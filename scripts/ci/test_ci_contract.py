from __future__ import annotations

import hashlib
import copy
import json
import os
import re
import shlex
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path
from urllib.parse import urlsplit

import yaml

ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".github" / "workflows"
SHA_ACTION = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")
CACHE_SHA = "27d5ce7f107fe9357f9df03efb73ab90386fccae"
R4_VERIFICATION_JOB = "r4-verification-build"
R4_TRIGGER = "${{ github.event_name == 'workflow_dispatch' && inputs.r4_verification_build }}"
R4_UPLOAD_ACTION = "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02"
R4_DELIVERY_PATHS = [
    "${{ runner.temp }}/r4-verification-build/delivery/verify_approved_simulation",
    "${{ runner.temp }}/r4-verification-build/delivery/provenance.json",
    "${{ runner.temp }}/r4-verification-build/delivery/owner-beta-simulation-runner",
    "${{ runner.temp }}/r4-verification-build/delivery/simulation-runner-provenance.json",
]
R4_PROOF_DELIVERY_PATHS = [
    "${{ runner.temp }}/r4-verification-build/proof-delivery/verify_approved_simulation_connection",
    "${{ runner.temp }}/r4-verification-build/proof-delivery/connection-proof-provenance.json",
]
R4_PROOF_UPLOAD_NAME = "r4-connection-proof-${{ github.sha }}"
CARGO_DOWNLOAD_PATHS = {
    "~/.cargo/registry/index", "~/.cargo/registry/cache", "~/.cargo/registry/src",
    "~/.cargo/git/db", "~/.cargo/git/checkouts",
}
CACHE_TARGETS = {
    "clippy": "${{ runner.temp }}/lagrange-cargo-target-clippy",
    "workspace-tests": "${{ runner.temp }}/lagrange-cargo-target-workspace",
    "postgres-integration-validation": "${{ runner.temp }}/lagrange-cargo-target-postgres",
    "research-smoke": "${{ runner.temp }}/lagrange-cargo-target-smoke",
}
CACHE_RUNTIME_ENVS = {
    "clippy": {"CARGO_TARGET_DIR": "lagrange-cargo-target-clippy"},
    "web": {"NPM_CONFIG_CACHE": "lagrange-npm-cache-web"},
    "workspace-tests": {
        "CARGO_TARGET_DIR": "lagrange-cargo-target-workspace",
        "UV_CACHE_DIR": "lagrange-uv-cache-workspace",
    },
    "python-tests": {"UV_CACHE_DIR": "lagrange-uv-cache-python"},
    "postgres-integration-validation": {"CARGO_TARGET_DIR": "lagrange-cargo-target-postgres"},
    "research-smoke": {"CARGO_TARGET_DIR": "lagrange-cargo-target-smoke"},
}
CACHE_FIRST_CONSUMERS = {
    "clippy": "rust-install",
    "web": "npm-cache-identity",
    "workspace-tests": "rust-install",
    "python-tests": "python-tools-install",
    "postgres-integration-validation": "rust-install",
    "research-smoke": "rust-install",
}


class UniqueBaseLoader(yaml.BaseLoader):
    def construct_mapping(self, node, deep=False):
        result = {}
        for key_node, value_node in node.value:
            key = self.construct_object(key_node, deep=deep)
            if key in result:
                raise AssertionError(f"duplicate workflow key: {key}")
            result[key] = self.construct_object(value_node, deep=deep)
        return result


def load_workflow(name: str) -> tuple[Path, dict]:
    path = WORKFLOWS / name
    if not path.is_file():
        raise AssertionError(f"missing workflow: {path.relative_to(ROOT)}")
    parsed = yaml.load(path.read_text(encoding="utf-8"), Loader=UniqueBaseLoader)
    if not isinstance(parsed, dict):
        raise AssertionError(f"workflow is not a mapping: {name}")
    return path, parsed


def action_uses(workflow: dict) -> list[str]:
    return [
        step["uses"]
        for job in workflow.get("jobs", {}).values()
        for step in job.get("steps", [])
        if "uses" in step
    ]


def command_lines(job: dict) -> list[str]:
    return [line.strip() for step in job["steps"] for line in step.get("run", "").splitlines()]


def assert_r4_verification_job(workflow: dict) -> None:
    dispatch = workflow["on"]["workflow_dispatch"]
    verification_input = dispatch["inputs"]["r4_verification_build"]
    assert verification_input == {
        "description": "Build the focused R4 verification binary from this source revision.",
        "required": "false",
        "default": "false",
        "type": "boolean",
    }, "R4 verification build must remain opt-in and default false"

    job = workflow["jobs"][R4_VERIFICATION_JOB]
    assert job.get("if") == R4_TRIGGER, "R4 verification job trigger broadened"
    assert job.get("permissions") == {"contents": "read"}, "R4 verification job permissions expanded"
    assert job.get("runs-on") == "ubuntu-latest", "R4 verification job runner changed"
    assert job.get("timeout-minutes") == "45", "R4 verification job timeout changed"
    assert set(job) == {
        "if", "permissions", "runs-on", "timeout-minutes", "steps"
    }, "R4 verification job gained inputs or execution scope"
    steps = job["steps"]
    assert len(steps) == 4, "R4 verification job steps changed"
    assert steps[0] == {
        "uses": "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
        "with": {"ref": "${{ github.sha }}", "persist-credentials": "false"},
    }, "R4 verification checkout must use the pinned, credential-free source revision"
    assert steps[1] == {
        "name": "Build and verify the source-only R4 binary",
        "run": "bash scripts/ci/build-r4-verification.sh",
    }, "R4 verification job build command or step environment changed"
    assert steps[2].get("name") == "Upload the R4 binary and provenance", (
        "original four-file R4 artifact must stay immediately after the build"
    )
    assert steps[3].get("name") == "Upload the R4 connection proof and provenance", (
        "the separate proof artifact must be the final R4 job step"
    )


def assert_artifact_upload_policy(workflow_name: str, workflow: dict) -> None:
    uploads = [
        (job_id, step)
        for job_id, job in workflow["jobs"].items()
        for step in job.get("steps", [])
        if step.get("uses", "").startswith("actions/upload-artifact@")
    ]
    if workflow_name != "ci.yml":
        assert not uploads, "artifact uploads are not approved in this workflow"
        return

    assert len(uploads) == 2, "only the two exact optional R4 artifacts may be uploaded"
    assert all(job_id == R4_VERIFICATION_JOB for job_id, _ in uploads), (
        "artifact upload moved outside the optional R4 source-only job"
    )
    job_steps = workflow["jobs"][R4_VERIFICATION_JOB]["steps"]
    original_step, proof_step = job_steps[-2:]
    assert uploads == [
        (R4_VERIFICATION_JOB, original_step),
        (R4_VERIFICATION_JOB, proof_step),
    ], "the original artifact must precede the separate proof artifact as the last two steps"
    assert original_step.get("name") == "Upload the R4 binary and provenance", (
        "original R4 artifact name or upload order changed"
    )
    assert original_step.get("uses") == R4_UPLOAD_ACTION, "unapproved artifact upload action"
    assert original_step.get("if") == "success()", "original artifact upload must be success-only"
    assert original_step.get("with") == {
        "name": "r4-verification-${{ github.sha }}",
        "path": "\n".join(R4_DELIVERY_PATHS) + "\n",
        "if-no-files-found": "error",
        "retention-days": "1",
        "compression-level": "0",
    }, "original R4 artifact name, fixed delivery paths, or retention policy changed"
    assert proof_step.get("name") == "Upload the R4 connection proof and provenance", (
        "proof artifact name or upload order changed"
    )
    assert proof_step.get("uses") == R4_UPLOAD_ACTION, "unapproved proof artifact upload action"
    assert proof_step.get("if") == "success()", "proof artifact upload must run only after successful verification"
    assert proof_step.get("with") == {
        "name": R4_PROOF_UPLOAD_NAME,
        "path": "\n".join(R4_PROOF_DELIVERY_PATHS) + "\n",
        "if-no-files-found": "error",
        "retention-days": "1",
        "compression-level": "0",
    }, "proof artifact name, fixed delivery paths, or retention policy changed"
    assert not any(
        candidate.get("uses", "").startswith("actions/download-artifact@")
        for candidate in job_steps
    ), "R4 verification job cannot consume an artifact input"


def assert_runner_env_context(workflow: dict) -> None:
    # runner is available in step env/with, but not workflow or job env.
    locations = [("workflow", workflow.get("env", {}))]
    locations.extend((f"job {job_id}", job.get("env", {})) for job_id, job in workflow["jobs"].items())
    for location, env in locations:
        for value in env.values():
            for expression in re.findall(r"\$\{\{(.*?)\}\}", value, re.DOTALL):
                expression = re.sub(r"'(?:[^']|'')*'", "", expression)
                assert not re.search(r"\brunner\b", expression, re.IGNORECASE), f"runner context unavailable in {location} env"


def assert_cache_runtime_env(job_id: str, job: dict) -> None:
    if job_id not in CACHE_RUNTIME_ENVS:
        return
    steps = job["steps"]
    initializer = next(step for step in steps if step.get("id") == "cache-epoch")
    script = initializer["run"]
    for variable, suffix in CACHE_RUNTIME_ENVS[job_id].items():
        assert variable not in job.get("env", {}), "cache path must be initialized at runtime"
        line = f"printf '{variable}=%s/{suffix}\\n' \"$RUNNER_TEMP\" >>\"$GITHUB_ENV\""
        assert line in script, "runtime cache path initializer missing or mismatched"
        assert script.index("\nfi\n") < script.index(line), "namespace validation must precede cache env writes"
    assert not re.search(r"\$(?:\{)?(?:CARGO_TARGET_DIR|NPM_CONFIG_CACHE|UV_CACHE_DIR)\b", script), "persisted cache env must not be consumed in its initializer step"
    consumer = next(step for step in steps if step.get("id") == CACHE_FIRST_CONSUMERS[job_id])
    assert steps.index(initializer) < steps.index(consumer), "cache env must be initialized before consumer"
    for step in steps:
        if step.get("uses", "").startswith("actions/cache/"):
            assert steps.index(initializer) < steps.index(step), "cache env must be initialized before consumer"


def assert_cache_policy(workflow: dict) -> None:
    assert_runner_env_context(workflow)
    for job_id, job in workflow["jobs"].items():
        assert job.get("permissions", {"contents": "read"}) == {"contents": "read"}, "job permissions expanded"
        ids = [step["id"] for step in job["steps"] if "id" in step]
        assert len(ids) == len(set(ids)), "duplicate step id"
        by_id = {step["id"]: step for step in job["steps"] if "id" in step}
        assert_cache_runtime_env(job_id, job)
        for step in job["steps"]:
            use = step.get("uses", "")
            if step.get("run"):
                assert "cache-hit" not in step.get("if", ""), "cache hit must not skip a command"
                assert step.get("continue-on-error") != "true", "check failure must not be masked"
            if use.startswith("actions/setup-node@"):
                assert "cache" not in step.get("with", {}), "duplicate implicit npm cache"
                assert step.get("with", {}).get("package-manager-cache") == "false", "automatic npm cache must be disabled"
            if not use.startswith("actions/cache"):
                continue
            assert use in {f"actions/cache/restore@{CACHE_SHA}", f"actions/cache/save@{CACHE_SHA}"}, "unapproved cache action"
            assert step.get("continue-on-error") == "true", "cache failures need real-command fallback"
            paths = set(step["with"]["path"].strip().splitlines())
            allowed = set(CARGO_DOWNLOAD_PATHS) | {
                *CACHE_TARGETS.values(),
                "${{ runner.temp }}/lagrange-npm-cache-web/_cacache",
                "${{ runner.temp }}/lagrange-uv-cache-workspace",
                "${{ runner.temp }}/lagrange-uv-cache-python",
                "${{ runner.temp }}/lagrange-smoke-build-cache/import/research-worker",
                "${{ runner.temp }}/lagrange-smoke-build-cache/import/candidate-runner",
            }
            assert paths and paths <= allowed, "cache path not allowlisted"
            if step["id"].startswith("cargo-download"):
                assert paths == CARGO_DOWNLOAD_PATHS, "public Cargo path set changed"
            if step["id"].startswith("cargo-target"):
                assert paths == {CACHE_TARGETS[job_id]}, "Cargo target is not job-private"
            kind = step["id"].rsplit("-", 1)[0]
            if kind not in {"cargo-download", "cargo-target"}:
                expected_paths = {
                    ("web", "npm"): "${{ runner.temp }}/lagrange-npm-cache-web/_cacache",
                    ("workspace-tests", "uv"): "${{ runner.temp }}/lagrange-uv-cache-workspace",
                    ("python-tests", "uv"): "${{ runner.temp }}/lagrange-uv-cache-python",
                    ("research-smoke", "docker-worker"): "${{ runner.temp }}/lagrange-smoke-build-cache/import/research-worker",
                    ("research-smoke", "docker-candidate"): "${{ runner.temp }}/lagrange-smoke-build-cache/import/candidate-runner",
                }
                assert (job_id, kind) in expected_paths, "cache kind/job not allowlisted"
                assert paths == {expected_paths[job_id, kind]}, "cache path belongs to another job or kind"
            assert step["with"].get("key"), "cache key missing"
            if "/save@" in use:
                restore_id = step["id"].removesuffix("-save") + "-restore"
                restore = by_id[restore_id]
                condition = step.get("if", "")
                assert condition.count("||") == 1, "save condition expands event/trust boundary"
                for required in (
                    "success()", "github.repository == 'sihioov/Lagrange'",
                    "github.event_name == 'push'", "github.event_name == 'workflow_dispatch'",
                    f"steps.{restore_id}.outcome == 'success'",
                    f"steps.{restore_id}.outputs.cache-hit != 'true'",
                ):
                    assert required in condition, "save trust/miss condition missing"
                assert step["with"]["path"] == restore["with"]["path"], "cold/warm path versions differ"
                assert step["with"]["key"] == restore["with"]["key"], "restore/save keys differ"
                assert "restore-keys" not in step["with"]
            else:
                assert step["with"].get("restore-keys"), "compatible restore prefix missing"
                assert "if" not in step, "restores must remain unconditional"


class WorkflowContractTests(unittest.TestCase):
    def test_r4_simulation_contract_pin_matches_document_sql_and_fixtures(self) -> None:
        contract_source = (
            ROOT / "crates/job-queue/src/owner_beta/simulation/contract.rs"
        ).read_text(encoding="utf-8")
        constant_match = re.search(
            r'pub const APPROVED_CONTRACT_SHA256:\s*&str\s*=\s*"(?P<value>sha256:[^"]+)"',
            contract_source,
        )
        self.assertIsNotNone(constant_match, "approved contract hash constant is missing")
        assert constant_match is not None
        approved_hash = constant_match.group("value")
        self.assertRegex(approved_hash, r"^sha256:[0-9a-f]{64}$")

        contract_document = (
            ROOT
            / "docs/superpowers/plans/2026-10-23-r4-price-only-simulation-contract-draft.md"
        )
        self.assertTrue(contract_document.is_file(), "adopted simulation contract document is missing")
        document_hash = "sha256:" + hashlib.sha256(contract_document.read_bytes()).hexdigest()
        self.assertEqual(approved_hash, document_hash)

        sql = (ROOT / "migrations/0063_owner_beta_price_only_simulations.up.sql").read_text(
            encoding="utf-8"
        )
        predicates = (
            (
                "run CHECK",
                re.compile(
                    r"CONSTRAINT\s+owner_beta_simulation_runs_contract_hash_check\s+"
                    r"CHECK\s*\(\s*contract_document_sha256\s*=\s*"
                    r"'(?P<value>sha256:[0-9a-f]+)'\s*\)",
                    re.DOTALL,
                ),
            ),
            (
                "jobs insert guard",
                re.compile(
                    r"payload_json\s*->>\s*'contract_document_sha256'\s+"
                    r"IS DISTINCT FROM\s*'(?P<value>sha256:[0-9a-f]+)'",
                    re.DOTALL,
                ),
            ),
        )

        def assert_contract_predicates(sql_text: str) -> None:
            for label, pattern in predicates:
                match = pattern.search(sql_text)
                if match is None:
                    raise AssertionError(f"missing {label} contract predicate")
                if match.group("value") != approved_hash:
                    raise AssertionError(f"{label} contract pin differs from approved document")

        assert_contract_predicates(sql)

        legacy_66_hex = (
            "sha256:8c5c6bf7f07de09f14f1e1e5eaa2b6479be4b10e8b98ed5a89a280ac95568e87cd"
        )
        self.assertEqual(len(legacy_66_hex.removeprefix("sha256:")), 66)
        for label, pattern in predicates:
            with self.subTest(mutated_predicate=label):
                match = pattern.search(sql)
                self.assertIsNotNone(match)
                assert match is not None
                start, end = match.span("value")
                mutant_sql = sql[:start] + legacy_66_hex + sql[end:]
                with self.assertRaisesRegex(AssertionError, re.escape(label)):
                    assert_contract_predicates(mutant_sql)

        fixture_value = re.compile(
            r'"contract_document_sha256"\s*:\s*'
            r"job_queue::owner_beta::simulation::APPROVED_CONTRACT_SHA256\b"
        )
        for fixture_path in (
            "crates/api-server/tests/common/mod.rs",
            "crates/job-queue/tests/queue_contract.rs",
        ):
            fixture_source = (ROOT / fixture_path).read_text(encoding="utf-8")
            self.assertRegex(
                fixture_source,
                fixture_value,
                f"{fixture_path} must use the exported approved contract hash",
            )

    def test_workspace_rustup_store_is_isolated_without_toolchain_override(self) -> None:
        _, ci = load_workflow("ci.yml")
        job = ci["jobs"]["workspace-tests"]
        self.assertNotIn("RUSTUP_HOME", job["env"])
        self.assertFalse(any("runner." in value for value in job["env"].values()))
        setup = [step for step in job["steps"]
                 if step.get("name") == "Prepare isolated Rust toolchain store"]
        self.assertEqual(len(setup), 1)
        self.assertEqual(job["steps"].index(setup[0]), 1)
        self.assertNotIn("if", setup[0])
        for env in [ci.get("env", {}), job["env"],
                    *(step.get("env", {}) for step in job["steps"])]:
            self.assertNotIn("RUSTUP_TOOLCHAIN", env)
            self.assertNotIn("RUSTUP_DIST_SERVER", env)
        self.assertNotIn("continue-on-error", job)
        commands = command_lines(job)
        self.assertIn("cargo test --workspace --locked --no-fail-fast --features kis-client/test-support", commands)
        self.assertIn("cargo test -p api-server --test http_owner_equity_v2_chart --locked", commands)
        for step in job["steps"]:
            if "continue-on-error" in step:
                self.assertIn(step.get("id"), {
                    "cargo-download-restore", "cargo-target-restore", "uv-restore",
                    "cargo-download-save", "cargo-target-save", "uv-save",
                })
            self.assertNotIn("RUSTUP_HOME", step.get("env", {}))

    def test_workspace_rustup_setup_creates_private_store_and_requires_runner_context(self) -> None:
        _, ci = load_workflow("ci.yml")
        script = next(step["run"] for step in ci["jobs"]["workspace-tests"]["steps"]
                      if step.get("name") == "Prepare isolated Rust toolchain store")
        with tempfile.TemporaryDirectory(prefix="rustup CI contract ") as root:
            env_path = Path(root) / "job-env"
            env = {"PATH": os.environ["PATH"], "RUNNER_TEMP": root,
                   "GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": "1",
                   "GITHUB_ENV": str(env_path)}
            result = subprocess.run(["bash", "-c", script], env=env, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            key, value = env_path.read_text().strip().split("=", 1)
            self.assertEqual(key, "RUSTUP_HOME")
            store = Path(value)
            self.assertEqual(store.parent, Path(root))
            self.assertTrue(store.name.startswith("lagrange-workspace-rustup-123-1."))
            self.assertEqual(store.stat().st_mode & 0o777, 0o700)
            self.assertEqual(list(store.iterdir()), [])
            original = env_path.read_bytes()
            del env["RUNNER_TEMP"]
            result = subprocess.run(["bash", "-c", script], env=env, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(env_path.read_bytes(), original)

    def test_grant_inspection_postgres_is_a_required_policy_step(self) -> None:
        _, ci = load_workflow("ci.yml")
        steps = [step for step in ci["jobs"]["policy"]["steps"]
                 if step.get("name") == "Verify market-stream grant inspection on isolated PostgreSQL"]
        self.assertEqual(len(steps), 1)
        self.assertEqual(steps[0]["run"],
            "sudo nice -n 10 /usr/bin/python3 -B scripts/ops/market-stream-grant-inspect-self-test.py")
        self.assertEqual(steps[0].get("if"), "${{ !cancelled() }}")
        self.assertNotIn("continue-on-error", steps[0])
        helper_steps = [step for step in ci["jobs"]["policy"]["steps"]
                        if step.get("name") == "Verify market-stream grant helper inspection boundaries"]
        self.assertEqual(len(helper_steps), 1)
        self.assertEqual(helper_steps[0]["run"],
                         "python -m unittest scripts.ops.test_market_stream_grant -v")
        self.assertNotIn("continue-on-error", helper_steps[0])
        required = ci["jobs"]["required"]
        self.assertIn("policy", required["needs"])
        names = [step.get("name") for step in ci["jobs"]["policy"]["steps"]]
        # The preceding transport fixture materializes this same pinned image;
        # the inspection fixture deliberately never pulls on its own.
        self.assertLess(names.index("Verify daily actual PostgreSQL control transport"),
                        names.index("Verify market-stream grant inspection on isolated PostgreSQL"))

    def test_daily_runner_status_bindings_are_a_required_policy_step(self) -> None:
        _, ci = load_workflow("ci.yml")
        steps = [step for step in ci["jobs"]["policy"]["steps"]
                 if step.get("name") == "Verify service-status binding for daily raw and action-supply modes"]
        self.assertEqual(len(steps), 1)
        self.assertEqual(steps[0]["run"],
                         "python -m unittest discover -s scripts/ops -p 'test*service_status*.py' -v")
        self.assertNotIn("if", steps[0])
        self.assertNotIn("continue-on-error", steps[0])

    def test_fixed_base_copy_principals_are_a_required_policy_step(self) -> None:
        _, ci = load_workflow("ci.yml")
        steps = [step for step in ci["jobs"]["policy"]["steps"]
                 if step.get("name") == "Verify fixed historical base copies and real reader principals"]
        self.assertEqual(len(steps), 1)
        self.assertEqual(steps[0]["run"], "sudo /usr/bin/python3 scripts/ops/owner-beta-daily-base-self-test.py")
        self.assertEqual(steps[0].get("if"), "${{ !cancelled() }}")
        self.assertNotIn("continue-on-error", steps[0])

    def test_wholeflow_docker_fixture_is_a_required_policy_step(self) -> None:
        _, ci = load_workflow("ci.yml")
        steps = [
            step for step in ci["jobs"]["policy"]["steps"]
            if step.get("name") == "Verify whole-flow actual Docker inherited volume boundaries"
        ]
        self.assertEqual(len(steps), 1)
        self.assertEqual(
            steps[0]["run"],
            "sudo /usr/bin/python3 scripts/ops/owner-beta-daily-wholeflow-docker-self-test.py",
        )
        self.assertEqual(steps[0].get("if"), "${{ !cancelled() }}")
        self.assertNotIn("continue-on-error", steps[0])

    def test_image_builder_fixture_is_a_required_policy_step(self) -> None:
        _, ci = load_workflow("ci.yml")
        steps = [
            step for step in ci["jobs"]["policy"]["steps"]
            if step.get("name") == "Verify complete image builder with synthetic Docker images"
        ]
        self.assertEqual(len(steps), 1)
        self.assertEqual(
            steps[0]["run"],
            "sudo env GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=safe.directory "
            'GIT_CONFIG_VALUE_0="$GITHUB_WORKSPACE" '
            "bash scripts/ops/build-production-images-self-test.sh",
        )
        self.assertEqual(steps[0].get("if"), "${{ !cancelled() }}")
        self.assertNotIn("continue-on-error", steps[0])

    def test_all_shell_scripts_are_checked_out_with_lf(self) -> None:
        attributes = (ROOT / ".gitattributes").read_text(encoding="utf-8").splitlines()
        self.assertIn("*.sh text eol=lf", attributes)

    def test_robustness_golden_files_are_portable_lf_artifacts(self) -> None:
        attributes = (ROOT / ".gitattributes").read_text(encoding="utf-8").splitlines()
        self.assertIn("tests/golden/robustness/**/*.json text eol=lf", attributes)

        golden_root = ROOT / "tests" / "golden" / "robustness"
        manifest = json.loads((golden_root / "golden-set.json").read_text("utf-8"))
        for artifact in manifest["artifacts"]:
            payload = (golden_root / artifact["path"]).read_bytes().replace(
                b"\r\n", b"\n"
            )
            actual = f"sha256:{hashlib.sha256(payload).hexdigest()}"
            self.assertEqual(actual, artifact["sha256"], artifact["id"])

    def test_triggers_have_no_schedule(self) -> None:
        _, ci = load_workflow("ci.yml")
        _, smoke = load_workflow("research-smoke.yml")
        self.assertEqual(set(ci["on"]), {"pull_request", "push", "workflow_dispatch"})
        self.assertEqual(ci["on"]["pull_request"]["branches"], ["main"])
        self.assertEqual(ci["on"]["push"]["branches"], ["main"])
        self.assertEqual(set(smoke["on"]), {"push", "workflow_dispatch"})
        self.assertEqual(smoke["on"]["push"]["branches"], ["main"])

    def test_actions_permissions_timeouts_and_storage_policy(self) -> None:
        for name in ("ci.yml", "research-smoke.yml"):
            path, workflow = load_workflow(name)
            self.assertEqual(workflow["permissions"], {"contents": "read"})
            for use in action_uses(workflow):
                self.assertRegex(use, SHA_ACTION)
                self.assertIn(use, {
                    "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
                    "actions/setup-node@2028fbc5c25fe9cf00d9f06a71cc4710d4507903",
                    "actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97",
                    R4_UPLOAD_ACTION,
                    f"actions/cache/restore@{CACHE_SHA}", f"actions/cache/save@{CACHE_SHA}",
                })
            for job in workflow["jobs"].values():
                self.assertEqual(job["runs-on"], "ubuntu-latest")
                self.assertIn("timeout-minutes", job)
                for step in job.get("steps", []):
                    if step.get("uses", "").startswith("actions/checkout@"):
                        self.assertEqual(
                            step.get("with", {}).get("persist-credentials"), "false"
                        )
            text = path.read_text(encoding="utf-8").lower()
            self.assertNotIn("target/", text)
            self.assertNotIn("pull_request_target", text)
            self.assertNotIn("secrets.", text)
            self.assertNotRegex(text, r"steps\.[a-z-]+-save\.outputs")
            assert_artifact_upload_policy(name, workflow)
            assert_cache_policy(workflow)

    def test_r4_verification_job_is_opt_in_and_source_only(self) -> None:
        _, workflow = load_workflow("ci.yml")
        assert_r4_verification_job(workflow)
        assert_artifact_upload_policy("ci.yml", workflow)

        script = (ROOT / "scripts/ci/build-r4-verification.sh").read_text(
            encoding="utf-8"
        )
        self.assertIn("set -euo pipefail", script)
        self.assertIn(
            "apk add --no-cache build-base musl-dev pkgconf openssl-dev postgresql-dev python3",
            script,
        )
        self.assertIn(
            "rust:1.97.1-alpine@sha256:3c38f3f82c2f3d73da3b38e18d279393a04cb43ddded0e35088a8c3324d40900",
            script,
        )
        self.assertIn('target=/source,readonly', script)
        self.assertIn('target=/out', script)
        self.assertIn("CARGO_BUILD_JOBS=2", script)
        self.assertIn("CARGO_TARGET_DIR=/out/cargo-target", script)
        self.assertIn("CARGO_INCREMENTAL=0", script)
        cargo_commands = [
            line.strip() for line in script.splitlines() if line.strip().startswith("cargo ")
        ]
        self.assertEqual(
            cargo_commands,
            [
                "cargo fmt --all --check",
                "cargo clippy --locked --release -p job-queue --example verify_approved_simulation -- -D warnings",
                "cargo test --locked --release -p job-queue --example verify_approved_simulation -- \\",
                "cargo clippy --locked --release -p job-queue --bin owner-beta-simulation-runner -- -D warnings",
                "cargo test --locked --release -p job-queue --lib owner_beta::simulation_execution::child::tests -- \\",
                "cargo test --locked --release -p job-queue --lib owner_beta::simulation_execution::provenance::tests -- \\",
                "cargo test --locked --release -p job-queue --lib owner_beta::simulation_execution::runner::tests -- \\",
                "cargo test --locked --release -p job-queue --bin owner-beta-simulation-runner -- \\",
                "cargo test --locked --release -p job-queue --test owner_beta_simulation_process -- \\",
                "cargo build --locked --release -p job-queue --example verify_approved_simulation",
                "cargo build --locked --release -p job-queue --bin owner-beta-simulation-runner",
                "cargo clippy --locked --release -p api-server --example verify_approved_simulation_connection -- -D warnings",
                "cargo test --locked --release -p api-server --example verify_approved_simulation_connection -- --nocapture --test-threads=1 \\",
                "cargo build --locked --release -p api-server --example verify_approved_simulation_connection",
            ],
        )
        for marker in (
            "rustc --version",
            "rustc -vV",
            "R4_VERIFICATION_GIT_COMMIT",
            "R4_VERIFICATION_TOOLCHAIN",
            "R4_VERIFICATION_TARGET",
            "R4_VERIFICATION_PROFILE=release",
            "R4_SIMULATION_BUILD_GIT_COMMIT",
            "R4_SIMULATION_BUILD_TOOLCHAIN",
            "R4_SIMULATION_BUILD_TARGET",
            "R4_SIMULATION_BUILD_PROFILE",
            "R4_SIMULATION_BUILD_WORKFLOW_RUN_ID",
            "R4_SIMULATION_BUILD_WORKFLOW_RUN_ATTEMPT",
            "R4_CONNECTION_PROOF_BUILD_GIT_COMMIT",
            "R4_CONNECTION_PROOF_BUILD_TOOLCHAIN",
            "R4_CONNECTION_PROOF_BUILD_TARGET",
            "R4_CONNECTION_PROOF_BUILD_PROFILE",
            "R4_CONNECTION_PROOF_BUILD_WORKFLOW_RUN_ID",
            "R4_CONNECTION_PROOF_BUILD_WORKFLOW_RUN_ATTEMPT",
            "readelf -l",
            "/out/delivery/verify_approved_simulation",
            "/out/delivery/owner-beta-simulation-runner",
            'simulation_provenance_path = delivery_dir / "simulation-runner-provenance.json"',
            '"owner-beta-simulation-build-v1"',
            '"owner-beta-simulation-connection-proof-build-v1"',
            'delivery_dir / "provenance.json"',
            "source_example_sha256",
            "source_cargo_lock_sha256",
            "source_job_queue_manifest_sha256",
            "simulation_source_paths",
            "simulation-source-hashes-before.tsv",
            "simulation-source-hashes-after.tsv",
            "R4_SOURCE_TEST_LOG:PASS",
            "/out/test-logs/",
            "verify-rust-test-log.py",
            "proof-source-inventory.py",
            "proof-source-paths-before.txt",
            "proof-source-paths-after.txt",
            "proof-source-hashes-before.tsv",
            "proof-source-hashes-after.tsv",
            "proof-build-metadata.txt",
            "proof-delivery",
            "R4_CONNECTION_PROOF_SOURCE_INVENTORY_BEGIN",
            "R4_CONNECTION_PROOF_RECEIPT_BEGIN",
            "R4_CONNECTION_PROOF_TEST_LOG:PASS",
            "R4_CONNECTION_PROOF_RECEIPT:PASS",
            "connection-proof-provenance.json",
            "proof_cli_rejects_missing_duplicate_unknown_and_nonisolated_options",
            "proof_receipts_bind_origin_compile_and_binaries",
            "proof_scratch_guard_requires_socket_nonce_and_empty_cluster",
            "proof_control_files_are_private_closed_and_no_overwrite",
            "proof_pair_rejects_mixed_binding_and_truncated_content",
            "proof_role_url_preserves_unix_socket_query",
            "GITHUB_RUN_ID",
            "GITHUB_RUN_ATTEMPT",
            "chmod 644 /out/build-metadata.txt /out/rustc-version.txt /out/rustc-vV.txt",
        ):
            self.assertIn(marker, script)
        for test_name in (
            "fixture_process_escalates_reaps_leader_and_leaves_no_live_group_member",
            "provenance_accepts_matching_receipt_compile_and_executable",
            "runner_runtime_boundary_is_linux_only",
            "ctrl_c_and_sigterm_request_the_same_cooperative_shutdown",
            "owner_beta_simulation_child_rejects_untrusted_startup_before_request_processing",
        ):
            self.assertIn(test_name, script)
        self.assertNotIn("/out/delivery/test-logs", script)
        self.assertIn(
            "expected_delivery_entries=$'owner-beta-simulation-runner\\nprovenance.json\\nsimulation-runner-provenance.json\\nverify_approved_simulation'",
            script,
        )
        self.assertNotIn("/var/run/docker.sock", script)
        self.assertNotIn("--secret", script)
        self.assertNotIn("&&", script)
        self.assertNotIn("actions/download-artifact", script)
        for forbidden in ("--workspace", "DATABASE_URL", "uv ", "phase0"):
            self.assertNotIn(forbidden, script)

    def test_r4_compiler_identity_accepts_matching_short_and_full_commit(self) -> None:
        script = (ROOT / "scripts/ci/build-r4-verification.sh").read_text()
        start = script.index('rustc_version="$(rustc --version)"')
        end = script.index('case "${R4_VERIFICATION_GIT_COMMIT:-}"', start)
        identity = script[start:end]
        full_commit = "8bab26f4f" + "a" * 31
        stub = "rustc() { if [ \"$1\" = --version ]; then printf '%s\\n' \"$R4_TEST_VERSION\"; else printf '%s\\n' \"$R4_TEST_VERBOSE\"; fi; }\n"
        verbose = f"release: 1.97.1\ncommit-hash: {full_commit}\nhost: x86_64-unknown-linux-musl"
        for version, expected_code in (
            ("rustc 1.97.1 (8bab26f4f 2026-07-14)", 0),
            ("rustc 1.97.1 (000000000 2026-07-14)", 2),
            ("rustc 1.97.0 (8bab26f4f 2026-07-14)", 2),
        ):
            with self.subTest(version=version):
                result = subprocess.run(
                    ["sh", "-eu", "-c", stub + identity], capture_output=True,
                    env={"PATH": os.defpath, "R4_TEST_VERSION": version, "R4_TEST_VERBOSE": verbose},
                    timeout=5, check=False,
                )
                self.assertEqual(result.returncode, expected_code, result.stderr)

    def test_r4_rust_test_log_parser_requires_each_successful_named_case(self) -> None:
        script = (ROOT / "scripts/ci/build-r4-verification.sh").read_text(
            encoding="utf-8"
        )
        start_marker = "# R4_SOURCE_TEST_LOG_PARSER_BEGIN\n"
        end_marker = "\nPYTHON\n# R4_SOURCE_TEST_LOG_PARSER_END"
        start = script.index("<<'PYTHON'\n", script.index(start_marker)) + len(
            "<<'PYTHON'\n"
        )
        end = script.index(end_marker, start)
        parser_source = script[start:end] + "\n"
        expected = ("first_case", "second_case")
        passing_log = "\n".join(
            (
                "running 2 tests",
                "test owner_beta::simulation_execution::child::tests::first_case ... ok",
                "test child::tests::second_case ... ok",
                "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 0.00s",
            )
        )
        failing_logs = {
            "zero": "running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s",
            "missing": "running 1 test\ntest tests::first_case ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s",
            "duplicate": "running 2 tests\ntest tests::first_case ... ok\ntest tests::first_case ... ok\ntest result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s",
            "failed": "running 2 tests\ntest tests::first_case ... ok\ntest tests::second_case ... FAILED\ntest result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s",
            "ignored": "running 2 tests\ntest tests::first_case ... ok\ntest tests::second_case ... ignored\ntest result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s",
            "skip": "SKIP: source-only lane was skipped\n" + passing_log,
        }
        with tempfile.TemporaryDirectory() as temporary:
            parser_path = Path(temporary) / "verify-rust-test-log.py"
            log_path = Path(temporary) / "test.log"
            parser_path.write_text(parser_source, encoding="utf-8")

            def invoke(contents: str, names: tuple[str, ...] = expected) -> subprocess.CompletedProcess:
                log_path.write_text(contents, encoding="utf-8")
                return subprocess.run(
                    [sys.executable, str(parser_path), str(log_path), "fixture", *names],
                    capture_output=True,
                    text=True,
                    env={"PATH": os.defpath},
                    timeout=5,
                    check=False,
                )

            accepted = invoke(passing_log)
            self.assertEqual(accepted.returncode, 0, accepted.stderr)
            self.assertIn("R4_SOURCE_TEST_LOG:PASS:fixture:2", accepted.stdout)
            for label, contents in failing_logs.items():
                with self.subTest(case=label):
                    rejected = invoke(contents)
                    self.assertNotEqual(rejected.returncode, 0)

    def test_r4_artifact_rejects_extra_data_log_and_wildcard_inputs(self) -> None:
        _, baseline = load_workflow("ci.yml")
        extra_paths = (
            "${{ runner.temp }}/r4-verification-build/delivery/build.log",
            "${{ runner.temp }}/r4-verification-build/delivery/provider-data.json",
            "${{ runner.temp }}/r4-verification-build/delivery/target/release/**",
            "${{ runner.temp }}/r4-verification-build/delivery/*.log",
        )
        for extra_path in extra_paths:
            changed = copy.deepcopy(baseline)
            changed["jobs"][R4_VERIFICATION_JOB]["steps"][-2]["with"]["path"] += (
                "\n" + extra_path
            )
            with self.subTest(path=extra_path), self.assertRaisesRegex(
                AssertionError, "original R4 artifact"
            ):
                assert_artifact_upload_policy("ci.yml", changed)

    def test_r4_proof_artifact_rejects_scope_and_order_changes(self) -> None:
        _, baseline = load_workflow("ci.yml")

        changed = copy.deepcopy(baseline)
        changed["jobs"][R4_VERIFICATION_JOB]["steps"][-1]["with"]["path"] += (
            "\n${{ runner.temp }}/r4-verification-build/proof-delivery/*.json"
        )
        with self.assertRaisesRegex(AssertionError, "proof artifact name, fixed delivery paths"):
            assert_artifact_upload_policy("ci.yml", changed)

        changed = copy.deepcopy(baseline)
        changed["jobs"][R4_VERIFICATION_JOB]["steps"][-1]["with"]["name"] = "wrong-proof-name"
        with self.assertRaisesRegex(AssertionError, "proof artifact name, fixed delivery paths"):
            assert_artifact_upload_policy("ci.yml", changed)

        changed = copy.deepcopy(baseline)
        changed["jobs"][R4_VERIFICATION_JOB]["steps"][-1].pop("if")
        with self.assertRaisesRegex(AssertionError, "proof artifact upload must run only"):
            assert_artifact_upload_policy("ci.yml", changed)

        changed = copy.deepcopy(baseline)
        steps = changed["jobs"][R4_VERIFICATION_JOB]["steps"]
        steps[-2], steps[-1] = steps[-1], steps[-2]
        with self.assertRaisesRegex(AssertionError, "original R4 artifact name or upload order"):
            assert_artifact_upload_policy("ci.yml", changed)

        changed = copy.deepcopy(baseline)
        original_upload = changed["jobs"][R4_VERIFICATION_JOB]["steps"][-2]
        original_upload["with"]["path"] += (
            "\n" + R4_PROOF_DELIVERY_PATHS[0]
        )
        with self.assertRaisesRegex(AssertionError, "original R4 artifact name"):
            assert_artifact_upload_policy("ci.yml", changed)

        changed = copy.deepcopy(baseline)
        changed["jobs"][R4_VERIFICATION_JOB]["steps"].append(
            copy.deepcopy(changed["jobs"][R4_VERIFICATION_JOB]["steps"][-1])
        )
        with self.assertRaisesRegex(AssertionError, "only the two exact optional R4"):
            assert_artifact_upload_policy("ci.yml", changed)

        changed = copy.deepcopy(baseline)
        proof_upload = changed["jobs"][R4_VERIFICATION_JOB]["steps"][-1]
        changed["jobs"]["policy"]["steps"].append(proof_upload)
        with self.assertRaisesRegex(AssertionError, "only the two exact optional R4"):
            assert_artifact_upload_policy("ci.yml", changed)

        changed = copy.deepcopy(baseline)
        moved_proof = changed["jobs"][R4_VERIFICATION_JOB]["steps"].pop()
        changed["jobs"]["policy"]["steps"].append(moved_proof)
        with self.assertRaisesRegex(AssertionError, "artifact upload moved outside"):
            assert_artifact_upload_policy("ci.yml", changed)

    def test_r4_proof_source_inventory_includes_declared_inputs_and_static_includes(self) -> None:
        script = (ROOT / "scripts/ci/build-r4-verification.sh").read_text(encoding="utf-8")
        marker = "# R4_CONNECTION_PROOF_SOURCE_INVENTORY_BEGIN\n"
        start = script.index("<<'PYTHON'\n", script.index(marker)) + len("<<'PYTHON'\n")
        end = script.index("\nPYTHON\nchmod 600 \"$proof_source_inventory_script\"", start)
        helper_source = script[start:end] + "\n"

        with tempfile.TemporaryDirectory(prefix="r4-proof-inventory-") as temporary:
            temp_root = Path(temporary)
            helper = temp_root / "proof-source-inventory.py"
            helper.write_text(helper_source, encoding="utf-8")

            def fixture_repo(name: str) -> Path:
                repo = temp_root / name
                repo.mkdir()
                files = {
                    "Cargo.toml": "[workspace]\n",
                    "Cargo.lock": "version = 4\n",
                    "rust-toolchain.toml": "[toolchain]\nchannel = \"1.97.1\"\n",
                    "README.md": "not a declared compile input\n",
                    "crates/api-server/Cargo.toml": "[package]\nname = \"fixture\"\n",
                    "crates/api-server/examples/verify.rs": (
                        'const BYTES: &[u8] = include_bytes!("fixtures/proof.bin");\n'
                        'const TEXT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/root.txt"));\n'
                    ),
                    "crates/api-server/examples/fixtures/proof.bin": b"fixture bytes",
                    "crates/api-server/fixtures/root.txt": "fixture text\n",
                    "migrations/0001_fixture.sql": "SELECT 1;\n",
                }
                for relative, content in files.items():
                    path = repo / relative
                    path.parent.mkdir(parents=True, exist_ok=True)
                    if isinstance(content, bytes):
                        path.write_bytes(content)
                    else:
                        path.write_text(content, encoding="utf-8")
                subprocess.run(["git", "init", "--quiet", str(repo)], check=True, capture_output=True)
                subprocess.run(["git", "-C", str(repo), "add", "--all"], check=True, capture_output=True)
                return repo

            repo = fixture_repo("valid")
            output = temp_root / "valid-inventory.txt"
            result = subprocess.run(
                [sys.executable, str(helper), str(repo), str(output)],
                capture_output=True, text=True, timeout=5, check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            paths = output.read_text(encoding="utf-8").splitlines()
            self.assertEqual(paths, sorted(paths))
            self.assertEqual(
                set(paths),
                {
                    "Cargo.toml", "Cargo.lock", "rust-toolchain.toml",
                    "crates/api-server/Cargo.toml",
                    "crates/api-server/examples/verify.rs",
                    "crates/api-server/examples/fixtures/proof.bin",
                    "crates/api-server/fixtures/root.txt",
                    "migrations/0001_fixture.sql",
                },
            )
            self.assertNotIn("README.md", paths)
            self.assertEqual(output.stat().st_mode & 0o777, 0o600)

            symlink_repo = fixture_repo("symlink")
            linked = symlink_repo / "crates/api-server/src/linked.rs"
            linked.parent.mkdir(parents=True, exist_ok=True)
            linked.symlink_to("../examples/verify.rs")
            subprocess.run(["git", "-C", str(symlink_repo), "add", "--all"], check=True, capture_output=True)
            symlink_output = temp_root / "symlink-inventory.txt"
            rejected_link = subprocess.run(
                [sys.executable, str(helper), str(symlink_repo), str(symlink_output)],
                capture_output=True, text=True, timeout=5, check=False,
            )
            self.assertNotEqual(rejected_link.returncode, 0)
            self.assertFalse(symlink_output.exists())

            newline_repo = fixture_repo("newline")
            newline_source = newline_repo / "bad\nsource.rs"
            newline_source.write_text("fn main() {}\n", encoding="utf-8")
            subprocess.run(["git", "-C", str(newline_repo), "add", "--all"], check=True, capture_output=True)
            newline_output = temp_root / "newline-inventory.txt"
            rejected_newline = subprocess.run(
                [sys.executable, str(helper), str(newline_repo), str(newline_output)],
                capture_output=True, text=True, timeout=5, check=False,
            )
            self.assertNotEqual(rejected_newline.returncode, 0)
            self.assertFalse(newline_output.exists())

    def test_r4_proof_source_hashes_reject_unsafe_paths_and_symlinks(self) -> None:
        script = (ROOT / "scripts/ci/build-r4-verification.sh").read_text(encoding="utf-8")
        marker = "# R4_CONNECTION_PROOF_SOURCE_HASH_BEGIN\n"
        start = script.index("<<'PYTHON'\n", script.index(marker)) + len("<<'PYTHON'\n")
        end = script.index("\nPYTHON\n# R4_CONNECTION_PROOF_SOURCE_HASH_END", start)
        helper_source = script[start:end] + "\n"
        with tempfile.TemporaryDirectory(prefix="r4-proof-hashes-") as temporary:
            temp_root = Path(temporary)
            helper = temp_root / "proof-source-hashes.py"
            helper.write_text(helper_source, encoding="utf-8")
            source_root = temp_root / "source"
            source_root.mkdir()
            source = source_root / "Cargo.toml"
            source.write_text("[workspace]\n", encoding="utf-8")
            inventory = temp_root / "paths.txt"
            output = temp_root / "hashes.tsv"

            def invoke() -> subprocess.CompletedProcess:
                return subprocess.run(
                    [sys.executable, str(helper), str(inventory), str(output), str(source_root)],
                    capture_output=True, text=True, timeout=5, check=False,
                )

            inventory.write_text("Cargo.toml\n", encoding="utf-8")
            result = invoke()
            self.assertEqual(result.returncode, 0, result.stderr)
            expected_digest = hashlib.sha256(source.read_bytes()).hexdigest()
            self.assertEqual(output.read_text(), f"Cargo.toml\t{expected_digest}\n")
            self.assertEqual(output.stat().st_mode & 0o777, 0o600)

            output.unlink()
            inventory.write_text("../Cargo.toml\n", encoding="utf-8")
            self.assertNotEqual(invoke().returncode, 0)
            self.assertFalse(output.exists())

            inventory.write_text("Cargo.toml\nCargo.toml\n", encoding="utf-8")
            self.assertNotEqual(invoke().returncode, 0)
            self.assertFalse(output.exists())

            link = source_root / "linked.rs"
            link.symlink_to(source)
            inventory.write_text("linked.rs\n", encoding="utf-8")
            self.assertNotEqual(invoke().returncode, 0)
            self.assertFalse(output.exists())

    def test_r4_proof_receipt_binds_identity_binary_and_stable_source_map(self) -> None:
        script = (ROOT / "scripts/ci/build-r4-verification.sh").read_text(encoding="utf-8")
        marker = "# R4_CONNECTION_PROOF_RECEIPT_BEGIN\n"
        start = script.index("<<'PYTHON'\n", script.index(marker)) + len("<<'PYTHON'\n")
        end = script.index("\nPYTHON\n# R4_CONNECTION_PROOF_RECEIPT_END", start)
        helper_source = script[start:end] + "\n"
        expected_fields = {
            "schema", "git_commit", "toolchain", "target", "profile",
            "workflow_run_id", "workflow_run_attempt", "binary_sha256", "source_hashes",
        }

        with tempfile.TemporaryDirectory(prefix="r4-proof-receipt-") as temporary:
            temp_root = Path(temporary)
            helper = temp_root / "proof-receipt.py"
            helper.write_text(helper_source, encoding="utf-8")

            def fixture(name: str, mutation: str | None = None):
                base = temp_root / name
                build_root = base / "build"
                proof_dir = build_root / "proof-delivery"
                delivery_dir = build_root / "delivery"
                source_root = base / "source"
                proof_dir.mkdir(mode=0o700, parents=True)
                proof_dir.chmod(0o700)
                delivery_dir.mkdir()
                source_root.mkdir()
                source_file = source_root / "Cargo.toml"
                source_file.write_text("[workspace]\n", encoding="utf-8")
                source_digest = hashlib.sha256(source_file.read_bytes()).hexdigest()
                paths = "Cargo.toml\n" if mutation != "unsafe-path" else "../Cargo.toml\n"
                for path_name in (
                    "proof-source-paths.txt", "proof-source-paths-before.txt",
                    "proof-source-paths-after.txt", "proof-source-paths-after-host.txt",
                ):
                    path = build_root / path_name
                    path.write_text(paths, encoding="utf-8")
                    path.chmod(0o600)
                hash_path = "Cargo.toml" if mutation != "unsafe-path" else "../Cargo.toml"
                hash_text = f"{hash_path}\t{source_digest}\n"
                for path_name in ("proof-source-hashes-before.tsv", "proof-source-hashes-after.tsv"):
                    path = build_root / path_name
                    path.write_text(hash_text, encoding="utf-8")
                    path.chmod(0o600)

                proof_binary = proof_dir / "verify_approved_simulation_connection"
                proof_bytes = b"\x7fELFproof-fixture"
                proof_binary.write_bytes(proof_bytes)
                proof_binary.chmod(0o755)
                runner_binary = delivery_dir / "owner-beta-simulation-runner"
                runner_bytes = b"\x7fELFrunner-fixture"
                runner_binary.write_bytes(runner_bytes)
                runner_binary.chmod(0o755)
                commit = "a" * 40
                run_id = "12345"
                attempt = "2"
                toolchain = "rustc-1.97.1-8bab26f4f"
                metadata_run = "12346" if mutation == "wrong-identity" else run_id
                metadata_digest = "0" * 64 if mutation == "wrong-proof-hash" else hashlib.sha256(proof_bytes).hexdigest()
                metadata = {
                    "git_commit": commit, "toolchain": toolchain,
                    "target": "x86_64-unknown-linux-musl", "profile": "release",
                    "workflow_run_id": metadata_run, "workflow_run_attempt": attempt,
                    "binary_sha256": metadata_digest, "binary_size_bytes": str(len(proof_bytes)),
                }
                metadata_path = build_root / "proof-build-metadata.txt"
                metadata_path.write_text("".join(f"{key}={value}\n" for key,value in metadata.items()), encoding="ascii")
                metadata_path.chmod(0o600)
                runner_receipt = {
                    "schema": "owner-beta-simulation-build-v1", "git_commit": commit,
                    "toolchain": toolchain, "target": "x86_64-unknown-linux-musl",
                    "profile": "release", "workflow_run_id": run_id,
                    "workflow_run_attempt": attempt,
                    "binary_sha256": hashlib.sha256(runner_bytes).hexdigest(),
                    "source_hashes": {"Cargo.toml": source_digest},
                }
                if mutation == "wrong-runner-binding":
                    runner_receipt["workflow_run_attempt"] = "3"
                runner_path = delivery_dir / "simulation-runner-provenance.json"
                runner_text = json.dumps(runner_receipt, sort_keys=True, separators=(",", ":"))
                if mutation == "duplicate-runner-key":
                    runner_text = runner_text.replace('"schema":', '"schema":"duplicate","schema":', 1)
                runner_path.write_text(runner_text + "\n", encoding="utf-8")
                runner_path.chmod(0o600)
                if mutation == "receipt-exists":
                    (proof_dir / "connection-proof-provenance.json").write_text("{}\n")
                    (proof_dir / "connection-proof-provenance.json").chmod(0o600)
                return build_root, proof_dir, runner_path, source_root, commit, run_id, attempt

            valid_args = fixture("valid")
            command = [sys.executable, str(helper), *(str(x) for x in valid_args[:4]), *valid_args[4:]]
            accepted = subprocess.run(command, capture_output=True, text=True, timeout=5, check=False)
            self.assertEqual(accepted.returncode, 0, accepted.stderr)
            receipt_path = valid_args[1] / "connection-proof-provenance.json"
            receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
            self.assertEqual(set(receipt), expected_fields)
            self.assertEqual(receipt["schema"], "owner-beta-simulation-connection-proof-build-v1")
            self.assertEqual(list(receipt["source_hashes"]), ["Cargo.toml"])
            self.assertEqual(receipt_path.stat().st_mode & 0o777, 0o600)

            for case in (
                "wrong-identity", "wrong-proof-hash", "wrong-runner-binding",
                "duplicate-runner-key", "unsafe-path", "receipt-exists",
            ):
                with self.subTest(case=case):
                    values = fixture(case, case)
                    args = [sys.executable, str(helper), *(str(x) for x in values[:4]), *values[4:]]
                    rejected = subprocess.run(args, capture_output=True, text=True, timeout=5, check=False)
                    self.assertNotEqual(rejected.returncode, 0)
                    if case != "receipt-exists":
                        self.assertFalse((values[1] / "connection-proof-provenance.json").exists())

    def test_r4_artifact_exception_rejects_scope_expansion(self) -> None:
        _, baseline = load_workflow("ci.yml")
        changed = copy.deepcopy(baseline)
        changed["jobs"]["policy"]["steps"].append(
            {"uses": R4_UPLOAD_ACTION, "with": {"path": "**/*"}}
        )
        with self.assertRaisesRegex(AssertionError, "only the two exact optional R4"):
            assert_artifact_upload_policy("ci.yml", changed)

        changed = copy.deepcopy(baseline)
        changed_upload = changed["jobs"][R4_VERIFICATION_JOB]["steps"][-2]
        changed_upload["with"]["path"] = "${{ runner.temp }}/r4-verification-build/**"
        with self.assertRaisesRegex(AssertionError, "original R4 artifact name"):
            assert_artifact_upload_policy("ci.yml", changed)

        changed = copy.deepcopy(baseline)
        changed["jobs"][R4_VERIFICATION_JOB]["steps"][-2].pop("if")
        with self.assertRaisesRegex(AssertionError, "original artifact upload must be success-only"):
            assert_artifact_upload_policy("ci.yml", changed)

        changed = copy.deepcopy(baseline)
        changed["jobs"][R4_VERIFICATION_JOB]["if"] = "${{ always() }}"
        with self.assertRaisesRegex(AssertionError, "trigger broadened"):
            assert_r4_verification_job(changed)

        changed = copy.deepcopy(baseline)
        changed["on"]["workflow_dispatch"]["inputs"]["r4_verification_build"][
            "default"
        ] = "true"
        with self.assertRaisesRegex(AssertionError, "default false"):
            assert_r4_verification_job(changed)

        _, smoke = load_workflow("research-smoke.yml")
        changed_smoke = copy.deepcopy(smoke)
        changed_smoke["jobs"][next(iter(changed_smoke["jobs"]))]["steps"].append(
            {"uses": R4_UPLOAD_ACTION, "with": {"path": "output"}}
        )
        with self.assertRaisesRegex(AssertionError, "not approved in this workflow"):
            assert_artifact_upload_policy("research-smoke.yml", changed_smoke)

    def test_cache_policy_rejects_path_expansion_gate_skips_and_untrusted_saves(self) -> None:
        _, baseline = load_workflow("ci.yml")
        for bad_path in ("~/.cargo", "~/.cargo/credentials.toml", "target/", "node_modules", "data/phase0", "${{ runner.temp }}/receipts"):
            changed = copy.deepcopy(baseline)
            step = next(step for step in changed["jobs"]["clippy"]["steps"] if step.get("id") == "cargo-download-restore")
            step["with"]["path"] = bad_path
            with self.subTest(path=bad_path), self.assertRaisesRegex(AssertionError, "not allowlisted"):
                assert_cache_policy(changed)
        changed = copy.deepcopy(baseline)
        step = next(step for step in changed["jobs"]["web"]["steps"] if step.get("id") == "npm-install")
        step["if"] = "steps.npm-restore.outputs.cache-hit != 'true'"
        with self.assertRaisesRegex(AssertionError, "must not skip"):
            assert_cache_policy(changed)
        for clause in ("github.repository == 'sihioov/Lagrange'", "steps.npm-restore.outputs.cache-hit != 'true'"):
            changed = copy.deepcopy(baseline)
            step = next(step for step in changed["jobs"]["web"]["steps"] if step.get("id") == "npm-save")
            step["if"] = step["if"].replace(clause, "true")
            with self.subTest(clause=clause), self.assertRaisesRegex(AssertionError, "trust/miss"):
                assert_cache_policy(changed)

    def test_workflow_and_job_env_reject_runner_context(self) -> None:
        for name in ("ci.yml", "research-smoke.yml"):
            _, baseline = load_workflow(name)
            assert_cache_policy(baseline)  # Step-level runner paths remain valid.
            for job_id in (None, *baseline["jobs"]):
                for value in ("${{ runner.temp }}", "${{ runner['temp'] }}", "${{ format('{0}', runner.temp) }}"):
                    changed = copy.deepcopy(baseline)
                    location = changed if job_id is None else changed["jobs"][job_id]
                    location.setdefault("env", {})["BAD_CACHE_PATH"] = value
                    with self.subTest(workflow=name, job=job_id, value=value), self.assertRaisesRegex(AssertionError, "runner context unavailable"):
                        assert_cache_policy(changed)
            literal = copy.deepcopy(baseline)
            literal["env"] = {"LITERAL": "runner.temp", "EXPRESSION_LITERAL": "${{ 'runner.temp' }}"}
            assert_cache_policy(literal)

    def test_cache_runtime_initialization_precedes_consumers(self) -> None:
        for name in ("ci.yml", "research-smoke.yml"):
            _, workflow = load_workflow(name)
            for job_id, job in workflow["jobs"].items():
                if job_id not in CACHE_RUNTIME_ENVS:
                    continue
                assert_cache_runtime_env(job_id, job)
                changed = copy.deepcopy(job)
                initializer = next(step for step in changed["steps"] if step.get("id") == "cache-epoch")
                initializer["run"] = initializer["run"].replace('>>"$GITHUB_ENV"', '>>"$GITHUB_OUTPUT"')
                with self.subTest(job=job_id, mutation="wrong destination"), self.assertRaisesRegex(AssertionError, "initializer missing or mismatched"):
                    assert_cache_runtime_env(job_id, changed)
                changed = copy.deepcopy(job)
                steps = changed["steps"]
                initializer = next(step for step in steps if step.get("id") == "cache-epoch")
                steps.remove(initializer)
                consumer = next(step for step in steps if step.get("id") == CACHE_FIRST_CONSUMERS[job_id])
                steps.insert(steps.index(consumer) + 1, initializer)
                with self.subTest(job=job_id, mutation="late initializer"), self.assertRaisesRegex(AssertionError, "before consumer"):
                    assert_cache_runtime_env(job_id, changed)
                changed = copy.deepcopy(job)
                initializer = next(step for step in changed["steps"] if step.get("id") == "cache-epoch")
                variable = next(iter(CACHE_RUNTIME_ENVS[job_id]))
                initializer["run"] += f'printf "%s" "${variable}"\n'
                with self.subTest(job=job_id, mutation="same step consumer"), self.assertRaisesRegex(AssertionError, "must not be consumed"):
                    assert_cache_runtime_env(job_id, changed)

    def test_cache_epoch_is_ascii_validated_through_environment(self) -> None:
        for name in ("ci.yml", "research-smoke.yml"):
            _, workflow = load_workflow(name)
            self.assertEqual(workflow["on"]["workflow_dispatch"]["inputs"]["cache_epoch"]["type"], "string")
            for job_id, job in workflow["jobs"].items():
                steps = job["steps"]
                if not any("actions/cache/restore@" in step.get("uses", "") for step in steps):
                    continue
                validator = next(step for step in steps if step.get("id") == "cache-epoch")
                self.assertIn("github.event.inputs.cache_epoch", validator["env"]["CACHE_EPOCH_INPUT"])
                self.assertIn("export LC_ALL=C", validator["run"])
                self.assertNotIn("${{", validator["run"])
                with tempfile.TemporaryDirectory(prefix="cache-epoch-") as temp:
                    sentinel = Path(temp) / "must-not-run"
                    output = Path(temp) / "output"
                    github_env = Path(temp) / "env"
                    runner_temp = Path(temp) / "runner temp"
                    for value in ("epoch_1.2-good", "", f"$(touch {sentinel})", "époch", "a\nvalue", "a" * 65):
                        output.write_text("")
                        github_env.write_text("")
                        env = {"PATH": os.defpath, "CACHE_EPOCH_INPUT": value, "GITHUB_OUTPUT": str(output), "GITHUB_ENV": str(github_env), "RUNNER_TEMP": str(runner_temp)}
                        result = subprocess.run(["bash", "-eu", "-c", validator["run"]], env=env, capture_output=True, text=True, timeout=5)
                        self.assertEqual(result.returncode, 0 if value == "epoch_1.2-good" else 2)
                        self.assertFalse(sentinel.exists())
                        if result.returncode:
                            self.assertEqual(github_env.read_text(), "", "invalid namespace wrote runtime cache env")
                            self.assertEqual(output.read_text(), "")
                            continue
                        expected = {variable: str(runner_temp / suffix) for variable, suffix in CACHE_RUNTIME_ENVS[job_id].items()}
                        self.assertEqual(github_env.read_text().splitlines(), [f"{variable}={path}" for variable, path in expected.items()])
                        self.assertEqual(output.read_text(), f"value={value}\n")
                        for variable, path in expected.items():
                            kind = {"CARGO_TARGET_DIR": "cargo-target", "NPM_CONFIG_CACHE": "npm", "UV_CACHE_DIR": "uv"}[variable]
                            restore = next(step for step in steps if step.get("id") == f"{kind}-restore")
                            cache_path = restore["with"]["path"].replace("${{ runner.temp }}", str(runner_temp))
                            self.assertEqual(cache_path, path + ("/_cacache" if variable == "NPM_CONFIG_CACHE" else ""))

    def test_preflight_lifecycle_and_hosted_powershell_parse(self) -> None:
        _, workflow = load_workflow("ci.yml")
        jobs = workflow["jobs"]
        for job_id in ("format", "clippy", "workspace-tests", "postgres-integration-validation"):
            commands = command_lines(jobs[job_id])
            install = "rustup toolchain install 1.97.1 --profile minimal --component rustfmt --component clippy"
            first_preflight = next(line for line in commands if "dev-check.py preflight" in line)
            self.assertLess(commands.index(install), commands.index(first_preflight), job_id)
        commands = command_lines(jobs["workspace-tests"])
        preflight = "python3 scripts/qa/dev-check.py preflight --profile rust --stage integration --environment ci"
        self.assertLess(commands.index("uv sync --project nt --locked"), commands.index(preflight))
        self.assertLess(commands.index("python scripts/ci/prepare_phase0.py --root data/phase0"), commands.index(preflight))
        self.assertLess(commands.index(preflight), commands.index("docker compose -f deploy/qa/qa-db.compose.yml up -d --wait"))
        policy_commands = "\n".join(command_lines(jobs["policy"]))
        self.assertIn("python -m unittest scripts.ci.test_ci_contract scripts.ci.test_cache_evidence -v", policy_commands)
        self.assertIn("python -m unittest discover -s scripts/qa -p 'test_dev_check.py' -v", policy_commands)
        parser = next(step for step in jobs["policy"]["steps"] if step.get("shell") == "pwsh")
        self.assertIn("Language.Parser]::ParseFile", parser["run"])
        self.assertNotIn("&", parser["run"])
        self.assertNotIn("verify-all.ps1", "\n".join(step.get("run", "") for step in jobs["policy"]["steps"] if step is not parser))

    def test_smoke_build_cache_keeps_private_builder_and_cold_probes(self) -> None:
        _, workflow = load_workflow("research-smoke.yml")
        text = (WORKFLOWS / "research-smoke.yml").read_text()
        self.assertIn("docker buildx create --driver docker-container --name", text)
        self.assertNotRegex(text, r"\s--use(?:\s|$)")
        self.assertNotIn("buildx prune", text)
        self.assertNotIn("system prune", text)
        for step in workflow["jobs"]["research-smoke"]["steps"]:
            if step.get("id") in {"docker-worker-save", "docker-candidate-save"}:
                self.assertIn("cache_promoted == 'true'", step["if"])
        script = (ROOT / "scripts/qa/research-worker-smoke.sh").read_text()
        self.assertIn('build_options=(--builder "$buildx_builder")', script)
        self.assertIn('if [ "${GITHUB_ACTIONS:-}" = true ]; then', script)
        self.assertIn("mode=max,ignore-error=true", script)
        self.assertIn("dkr build --no-cache", script)
        probe = script.split("raw_init_ownership_probe() (", 1)[1].split("raw_init_ownership_probe\n", 1)[0]
        self.assertEqual(len(re.findall(r"^  dkr volume create ", probe, re.MULTILINE)), 6)

    def test_smoke_builder_identity_accepts_buildx_labels_and_rejects_unknown_versions(self) -> None:
        _, workflow = load_workflow("research-smoke.yml")
        step = next(step for step in workflow["jobs"]["research-smoke"]["steps"] if step.get("id") == "buildx")
        fake_docker = '''#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
with open(os.environ["FAKE_DOCKER_LOG"], "a", encoding="utf-8") as log:
    log.write(json.dumps(args) + "\\n")
if args == ["buildx", "create", "--driver", "docker-container", "--name", "fixture-builder"]:
    print("fixture-builder")
elif args == ["buildx", "inspect", "fixture-builder", "--bootstrap"]:
    print(os.environ["FAKE_INSPECTION"])
elif args == ["version", "--format", "{{.Server.Version}}"]:
    print("28.5.1")
elif args == ["compose", "version", "--short"]:
    print("2.39.0")
elif args == ["buildx", "version"]:
    print("github.com/docker/buildx v0.36.1 fixture")
else:
    raise SystemExit("unexpected fake Docker invocation")
'''
        cases = {
            "legacy": ("BuildKit:       v0.33.1\n", True),
            "current": ("BuildKit version:  v0.33.1\n", True),
            "prerelease": ("  BuildKit version: v0.33.1-rc1\n", True),
            "missing": ("Status: running\n", False),
            "empty": ("BuildKit version: \n", False),
            "malformed": ("BuildKit version: unknown\n", False),
            "lookalike": ("OtherBuildKit version: v0.33.1\n", False),
            "ambiguous": ("BuildKit: v0.33.1\nBuildKit version: v0.34.0\n", False),
        }
        identities = {}
        with tempfile.TemporaryDirectory(prefix="builder-identity-") as temp:
            root = Path(temp)
            docker = root / "docker"
            docker.write_text(fake_docker)
            docker.chmod(0o700)
            for name, (inspection, expected_success) in cases.items():
                with self.subTest(case=name):
                    output, calls = root / "output", root / "calls"
                    output.write_text("")
                    calls.write_text("")
                    env = {
                        "PATH": str(root) + os.pathsep + os.defpath,
                        "RESEARCH_SMOKE_BUILDER": "fixture-builder",
                        "GITHUB_OUTPUT": str(output),
                        "FAKE_DOCKER_LOG": str(calls),
                        "FAKE_INSPECTION": "Name: fixture-builder\nDriver: docker-container\nNodes:\n" + inspection,
                        "ImageOS": "ubuntu24",
                        "ImageVersion": "fixture",
                    }
                    result = subprocess.run(["bash", "-eu", "-c", step["run"]], env=env, capture_output=True, text=True, timeout=5)
                    self.assertEqual(result.returncode, 0 if expected_success else 1, result.stderr)
                    invocations = [json.loads(line) for line in calls.read_text().splitlines()]
                    self.assertEqual(invocations[:2], [
                        ["buildx", "create", "--driver", "docker-container", "--name", "fixture-builder"],
                        ["buildx", "inspect", "fixture-builder", "--bootstrap"],
                    ])
                    if expected_success:
                        values = dict(line.split("=", 1) for line in output.read_text().splitlines())
                        self.assertEqual(values["builder"], "fixture-builder")
                        self.assertRegex(values["identity"], r"^[0-9a-f]{64}$")
                        self.assertEqual(len(invocations), 5)
                        identities[name] = values["identity"]
                    else:
                        self.assertEqual(output.read_text(), "")
                        self.assertEqual(len(invocations), 2)
                        self.assertIn("exactly one BuildKit version", result.stderr)
        self.assertEqual(identities["legacy"], identities["current"])
        self.assertNotEqual(identities["current"], identities["prerelease"])

    def test_smoke_cache_ignores_missing_export_and_promotes_only_valid_layout(self) -> None:
        source = (ROOT / "scripts/qa/research-worker-smoke.sh").read_text()
        functions = source.split("configure_service_build_cache() {", 1)[1].split('context_audit_tag="', 1)[0]
        functions = "configure_service_build_cache() {" + functions
        with tempfile.TemporaryDirectory(prefix="smoke-cache-functions-") as temp:
            root = Path(temp)
            imported = root / "import/research-worker"
            exported = root / "new/research-worker"
            imported.mkdir(parents=True)
            exported.mkdir(parents=True)
            override = root / "override.json"
            preamble = (
                "set -euo pipefail\nhosted_build_cache=1\nresearch_worker_cache_promoted=false\nresearch_worker_cache_bytes=unavailable\n"
                f"import_root={shlex.quote(str(root / 'import'))}\nnew_root={shlex.quote(str(root / 'new'))}\ncompose_override={shlex.quote(str(override))}\n"
            )
            def execute(body):
                return subprocess.run(["bash", "-c", preamble + functions + body], capture_output=True, text=True, timeout=5, check=True)
            result = execute("configure_service_build_cache research-worker\npromote_service_build_cache research-worker\nprintf '%s' \"$research_worker_cache_promoted\"\n")
            self.assertEqual(result.stdout, "false")
            self.assertNotIn("cache_from", json.loads(override.read_text())["services"]["research-worker"]["build"])
            old_index = '{"manifests":[{"digest":"sha256:old"}]}\n'
            imported.joinpath("index.json").write_text(old_index)
            imported.joinpath("blobs/sha256").mkdir(parents=True)
            imported.joinpath("blobs/sha256/old").write_text("old")
            execute("configure_service_build_cache research-worker\n")
            build = json.loads(override.read_text())["services"]["research-worker"]["build"]
            self.assertEqual(build["cache_from"], [f"type=local,src={imported}"])
            self.assertEqual(imported.joinpath("index.json").read_text(), old_index)
            new_index = '{"manifests":[{"digest":"sha256:new"}]}\n'
            exported.joinpath("index.json").write_text(new_index)
            exported.joinpath("blobs/sha256").mkdir(parents=True)
            exported.joinpath("blobs/sha256/new").write_text("new")
            result = execute("promote_service_build_cache research-worker\nprintf '%s' \"$research_worker_cache_promoted\"\n")
            self.assertEqual(result.stdout, "true")
            self.assertFalse(imported.joinpath("blobs/sha256/old").exists())
            self.assertTrue(imported.joinpath("blobs/sha256/new").exists())
            self.assertEqual(imported.joinpath("index.json").read_text(), new_index)
            self.assertFalse(exported.exists())

    def _run_smoke_bake_helper(self, case="valid", service="research-worker", warm=False, sequence=False):
        source = (ROOT / "scripts/qa/research-worker-smoke.sh").read_text()
        functions = "configure_service_build_cache() {" + source.split("configure_service_build_cache() {", 1)[1].split('context_audit_tag="', 1)[0]
        fake = '''import json, os, pathlib, sys
kind, *args = sys.argv[1:]
entry = {"kind": kind, "argv": args}
if kind == "dkr":
    entry["printed"] = json.loads(pathlib.Path(args[args.index("--file") + 1]).read_text())
with open(os.environ["FAKE_LOG"], "a") as log:
    log.write(json.dumps(entry) + "\\n")
case = os.environ["FAKE_CASE"]
if kind == "dkr":
    raise SystemExit(42 if case == "bake-failure" else 0)
if "--print" not in args:
    raise SystemExit(0)
if case == "print-failure":
    raise SystemExit(41)
if case == "malformed":
    print("{")
    raise SystemExit(0)
service = args[-1]
override_path = pathlib.Path(os.environ["FAKE_OVERRIDE"])
override = json.loads(override_path.read_text())
build = override["services"][service]["build"]
target = {"context": "/fixture/context", "dockerfile": "/fixture/context/Dockerfile", "tags": ["fixture/" + service + ":local"], "args": {"LAGRANGE_CODE_COMMIT": "0" * 40}, "labels": {"fixture": "preserved"}, "cache-to": build["cache_to"], "output": ["type=docker"]}
if "cache_from" in build:
    target["cache-from"] = build["cache_from"]
value = {"group": {"default": {"targets": [service]}}, "target": {service: target}}
if case in ("auxiliary", "compose-2.38"):
    value["target"].update({"db-migrate": {"context": "/unused"}, "db-role-bootstrap": {"context": "/unused"}})
if case == "compose-2.38": target["output"] = ["type=docker,load=true,push=false"]
if case == "auxiliary-group":
    value["target"]["db-migrate"] = dict(target)
    value["group"]["default"]["targets"].append("db-migrate")
if case == "other-rust": value["target"]["candidate-runner" if service == "research-worker" else "research-worker"] = dict(target)
if case == "inherits": target["inherits"] = ["db-migrate"]
if case == "target-stage": target["target"] = "target:db-migrate"
if case == "bad-target": value["target"][service] = []
if case == "bad-root": value = None
if case == "duplicate":
    print('{"group":{},"group":{},"target":{}}')
    raise SystemExit(0)
if case == "extra-target": value["target"]["other"] = dict(target)
if case == "extra-group": value["group"]["default"]["targets"].append("other")
if case == "dependency": target["contexts"] = {"other": "target:other"}
if case == "target-context": target["context"] = "target:other"
if case == "bad-export": target["cache-to"] = ["type=local,dest=/unapproved,mode=max,ignore-error=true"]
if case in ("bad-import", "unexpected-import"): target["cache-from"] = ["type=local,src=/unapproved"]
if case == "push": target["output"] = ["type=image,push=true"]
if case == "docker-push": target["output"] = ["type=docker,load=true,push=true"]
if case == "docker-dest": target["output"] = ["type=docker,load=true,push=false,dest=/unapproved"]
if case == "multiple-outputs": target["output"] = ["type=docker,load=true,push=false", "type=registry"]
if case == "file-output": target["output"] = ["type=local,dest=/unapproved"]
if case == "privilege": target["entitlements"] = ["security.insecure", "network.host"]
if case == "bad-args": target["args"] = False
if case == "bad-override":
    override["services"]["other"] = {}
    override_path.write_text(json.dumps(override))
if case == "objects":
    for key in ("cache-to", "cache-from"):
        if key in target: target[key] = [dict(item.split("=", 1) for item in value.split(",")) for value in target[key]]
    target["output"] = [{"type": "docker"}]
print(json.dumps(value))
'''
        with tempfile.TemporaryDirectory(prefix="smoke-bake-helper-") as temp:
            root = Path(temp)
            work = root / "work"
            work.mkdir(mode=0o700)
            imported, exported = root / "cache/import", root / "cache/new"
            if case == "wildcard":
                exported = root / "cache/new*"
            if warm:
                for name in ("research-worker", "candidate-runner"):
                    cache = imported / name
                    (cache / "blobs").mkdir(parents=True)
                    (cache / "index.json").write_text('{"manifests":[{}]}')
            fake_path, log, override = root / "fake.py", root / "calls.jsonl", work / "override.json"
            fake_path.write_text(fake)
            env = {"PATH": os.defpath, "FAKE_LOG": str(log), "FAKE_CASE": case, "FAKE_OVERRIDE": str(override)}
            local = case == "local"
            preamble = (
                "set -euo pipefail\n"
                f"temp_root={shlex.quote(str(work))}\ncompose_override={shlex.quote(str(override))}\n"
                f"import_root={shlex.quote(str(imported))}\nnew_root={shlex.quote(str(exported))}\n"
                f"hosted_build_cache={0 if local else 1}\nbuildx_builder=fixture-builder\n"
                + ("build_options=()\n" if local else "build_options=(--builder fixture-builder)\n")
                + "research_worker_cache_promoted=false\ncandidate_runner_cache_promoted=false\nbuild_seconds=0\n"
                + f"rc() {{ python3 {shlex.quote(str(fake_path))} rc \"$@\"; }}\n"
                + f"dkr() {{ python3 {shlex.quote(str(fake_path))} dkr \"$@\"; }}\n"
                + "fail() { exit 1; }\n"
            )
            if sequence:
                body = "if ! configure_service_build_cache research-worker; then" + source.split("if ! configure_service_build_cache research-worker; then", 1)[1].split("command -v cargo", 1)[0]
            else:
                body = "" if case == "unavailable" else "configure_service_build_cache research-worker\n"
                if service == "candidate-runner":
                    body = "configure_service_build_cache candidate-runner\n"
                body += f"build_service_image {shlex.quote(service)}\n"
            result = subprocess.run(["bash", "-c", preamble + functions + body], env=env, capture_output=True, text=True, timeout=5)
            calls = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
            printed = {p.name: p.read_text() for p in work.glob("*.bake.json")}
            return result, calls, printed, str(work), str(imported), str(exported)

    def test_smoke_bake_build_grants_only_service_cache_paths(self) -> None:
        for service in ("research-worker", "candidate-runner"):
            for warm in (False, True):
                with self.subTest(service=service, warm=warm):
                    result, calls, printed, work, imported, exported = self._run_smoke_bake_helper(service=service, warm=warm)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(calls[0], {"kind": "rc", "argv": ["build", "--print", service]})
                    expected = ["buildx", "bake", "--builder", "fixture-builder", "--file", f"{work}/build-{service}.bake.json", "--load", "--allow", f"fs.write={exported}/{service}"]
                    if warm:
                        expected += ["--allow", f"fs.read={imported}/{service}"]
                    self.assertEqual(calls[1]["argv"], expected + [service])
                    self.assertEqual(len(calls), 2)
                    value = json.loads(printed[f"build-{service}.bake.json"])
                    self.assertEqual(value, calls[1]["printed"])
                    self.assertEqual(printed[f"build-{service}.bake.json"], json.dumps(value) + "\n")
                    self.assertEqual(value["target"][service]["context"], "/fixture/context")
                    self.assertEqual(value["target"][service]["dockerfile"], "/fixture/context/Dockerfile")
                    self.assertEqual(value["target"][service]["args"], {"LAGRANGE_CODE_COMMIT": "0" * 40})
                    self.assertEqual(value["target"][service]["labels"], {"fixture": "preserved"})
                    self.assertEqual(value["target"][service]["tags"], [f"fixture/{service}:local"])
        result, calls, *_ = self._run_smoke_bake_helper(case="objects", warm=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 2)

    def test_smoke_bake_build_filters_only_known_unselected_auxiliaries(self) -> None:
        for service in ("research-worker", "candidate-runner"):
            for warm in (False, True):
                with self.subTest(service=service, warm=warm):
                    result, calls, printed, work, imported, exported = self._run_smoke_bake_helper(case="auxiliary", service=service, warm=warm)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    value = calls[1]["printed"]
                    self.assertEqual(value["group"], {"default": {"targets": [service]}})
                    expected_target = {"context": "/fixture/context", "dockerfile": "/fixture/context/Dockerfile", "tags": [f"fixture/{service}:local"], "args": {"LAGRANGE_CODE_COMMIT": "0" * 40}, "labels": {"fixture": "preserved"}, "cache-to": [f"type=local,dest={exported}/{service},mode=max,ignore-error=true"], "output": ["type=docker"]}
                    if warm:
                        expected_target["cache-from"] = [f"type=local,src={imported}/{service}"]
                    self.assertEqual(value["target"], {service: expected_target})
                    argv = ["buildx", "bake", "--builder", "fixture-builder", "--file", f"{work}/build-{service}.bake.json", "--load", "--allow", f"fs.write={exported}/{service}"]
                    if warm:
                        argv += ["--allow", f"fs.read={imported}/{service}"]
                    self.assertEqual(calls[1]["argv"], argv + [service])
                    self.assertEqual(len(calls), 2)
                    self.assertEqual(json.loads(printed[f"build-{service}.bake.json"]), value)

    def test_smoke_bake_build_preserves_compose_2_38_local_load_output(self) -> None:
        for service in ("research-worker", "candidate-runner"):
            for warm in (False, True):
                with self.subTest(service=service, warm=warm):
                    result, calls, printed, work, imported, exported = self._run_smoke_bake_helper(case="compose-2.38", service=service, warm=warm)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    value = calls[1]["printed"]
                    self.assertEqual(value["group"], {"default": {"targets": [service]}})
                    expected_target = {"context": "/fixture/context", "dockerfile": "/fixture/context/Dockerfile", "tags": [f"fixture/{service}:local"], "args": {"LAGRANGE_CODE_COMMIT": "0" * 40}, "labels": {"fixture": "preserved"}, "cache-to": [f"type=local,dest={exported}/{service},mode=max,ignore-error=true"], "output": ["type=docker,load=true,push=false"]}
                    if warm:
                        expected_target["cache-from"] = [f"type=local,src={imported}/{service}"]
                    self.assertEqual(value["target"], {service: expected_target})
                    argv = ["buildx", "bake", "--builder", "fixture-builder", "--file", f"{work}/build-{service}.bake.json", "--load", "--allow", f"fs.write={exported}/{service}"]
                    if warm:
                        argv += ["--allow", f"fs.read={imported}/{service}"]
                    self.assertEqual(calls[1]["argv"], argv + [service])
                    self.assertEqual(len(calls), 2)
                    self.assertEqual(json.loads(printed[f"build-{service}.bake.json"]), value)

    def test_smoke_bake_build_rejects_unapproved_graph_paths_and_outputs(self) -> None:
        codes = {"malformed": "parse-or-type", "extra-target": "target-set", "extra-group": "default-group", "dependency": "target-fields", "target-context": "target-context", "bad-export": "target-export", "bad-import": "target-import", "unexpected-import": "target-import", "push": "target-output", "file-output": "target-output", "privilege": "target-fields", "bad-args": "target-mapping", "bad-override": "override-service", "wildcard": "cache-path", "auxiliary-group": "default-group", "other-rust": "target-set", "inherits": "target-fields", "target-stage": "target-stage", "bad-target": "target-fields", "bad-root": "parse-or-type", "duplicate": "duplicate-key"}
        codes.update({"docker-push": "target-output", "docker-dest": "target-output", "multiple-outputs": "target-output"})
        for case, code in codes.items():
            with self.subTest(case=case):
                result, calls, *_ = self._run_smoke_bake_helper(case=case, warm=case == "bad-import")
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertEqual(result.stderr.strip(), f"RESEARCH_WORKER_SMOKE: printed Bake definition rejected [{code}]")
                self.assertFalse(any(call["kind"] == "dkr" for call in calls))
        result, calls, *_ = self._run_smoke_bake_helper(service="unknown-service")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(calls, [])

    def test_smoke_bake_build_preserves_failures_and_local_fallback(self) -> None:
        for case, exit_code in (("print-failure", 41), ("bake-failure", 42)):
            result, calls, *_ = self._run_smoke_bake_helper(case=case)
            self.assertEqual(result.returncode, exit_code)
            self.assertEqual(len(calls), 1 if case == "print-failure" else 2)
            result, calls, *_ = self._run_smoke_bake_helper(case=case, sequence=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(any("candidate-runner" in call["argv"] for call in calls))
        result, calls, *_ = self._run_smoke_bake_helper(sequence=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call["argv"][-1] for call in calls], ["research-worker", "research-worker", "candidate-runner", "candidate-runner"])
        for case, args in (("local", ["build", "research-worker"]), ("unavailable", ["build", "--builder", "fixture-builder", "research-worker"])):
            result, calls, *_ = self._run_smoke_bake_helper(case=case)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(calls, [{"kind": "rc", "argv": args}])

    def test_ci_runs_required_gates_and_disposable_data(self) -> None:
        path, ci = load_workflow("ci.yml")
        self.assertEqual(
            set(ci["jobs"]),
            {
                "policy",
                "format",
                "clippy",
                "web",
                "workspace-tests",
                "python-tests",
                "postgres-integration-validation",
                R4_VERIFICATION_JOB,
                "required",
            },
        )
        text = path.read_text(encoding="utf-8")
        self.assertIn("scripts/ci/prepare_phase0.py --root data/phase0", text)
        self.assertIn("deploy/qa/qa-db.compose.yml up -d --wait", text)
        workspace_test_command = (
            "cargo test --workspace --locked --no-fail-fast "
            "--features kis-client/test-support"
        )
        self.assertIn(workspace_test_command, text)
        self.assertIn(
            "bash deploy/db/integration-validation/static-check.sh", text
        )
        self.assertIn(
            "bash deploy/db/integration-validation/validate.sh --self-test", text
        )
        self.assertIn("bash deploy/compose/candidate-static-check.sh", text)
        self.assertIn(
            'bash deploy/db/integration-validation/validate.sh --evidence-dir "$evidence_dir"',
            text,
        )
        self.assertIn(
            "cargo clippy --workspace --all-targets --all-features --locked -- -D warnings",
            text,
        )
        self.assertIn("cargo fmt --all -- --check", text)
        self.assertIn("CARGO_INCREMENTAL: '0'", text)
        expected_web_commands = [
            "npm ci", "npx playwright install --with-deps chromium",
            "npm run openapi:check --workspace @lagrange/api-server",
            "npm run lint --workspace @lagrange/web", "npm run typecheck --workspace @lagrange/web",
            "npm test --workspace @lagrange/web", "npm run build --workspace @lagrange/web",
            "bash scripts/qa/candidate-web-e2e.sh",
        ]
        web_commands = [line for line in command_lines(ci["jobs"]["web"])
                        if line in expected_web_commands or line.startswith("node scripts/qa/")]
        self.assertEqual(
            web_commands,
            [
                "npm ci",
                "npx playwright install --with-deps chromium",
                "npm run openapi:check --workspace @lagrange/api-server",
                "npm run lint --workspace @lagrange/web",
                "npm run typecheck --workspace @lagrange/web",
                "npm test --workspace @lagrange/web",
                'node scripts/qa/recommendation-poll-recovery-browser.mjs '
                '"/tmp/lagrange-recommendation-poll-${{ github.run_id }}-${{ github.run_attempt }}"',
                'node scripts/qa/recommendation-submit-lifetime-browser.mjs '
                '"/tmp/lagrange-recommendation-submit-${{ github.run_id }}-${{ github.run_attempt }}"',
                'node scripts/qa/daily-preference-lifetime-browser.mjs '
                '"/tmp/lagrange-daily-preference-${{ github.run_id }}-${{ github.run_attempt }}"',
                'node scripts/qa/stock-beta-membership-order-browser.mjs '
                '"/tmp/lagrange-membership-order-${{ github.run_id }}-${{ github.run_attempt }}"',
                "npm run build --workspace @lagrange/web",
                "bash scripts/qa/candidate-web-e2e.sh",
            ],
        )
        self.assertLess(
            web_commands.index("npx playwright install --with-deps chromium"),
            web_commands.index("npm test --workspace @lagrange/web"),
        )
        self.assertLess(
            web_commands.index("npx playwright install --with-deps chromium"),
            web_commands.index("bash scripts/qa/candidate-web-e2e.sh"),
        )
        playwright_step = next(
            step
            for step in ci["jobs"]["web"]["steps"]
            if "npx playwright install --with-deps chromium" in step.get("run", "").splitlines()
        )
        self.assertEqual(playwright_step.get("working-directory"), "apps/web")
        policy_steps = ci["jobs"]["policy"]["steps"]
        independent_fixtures = {
            "Verify daily backup files and failure ordering",
            "Verify daily encrypted full-schema isolated restore",
            "Verify installed backup version transition and recovery",
            "Verify pre-upgrade cold archive boundaries",
            "Verify pre-upgrade maintenance failure boundaries",
            "Verify pre-upgrade schema59 isolated recovery",
            "Verify whole-flow fixture and driver boundaries",
            "Verify whole-flow actual Docker inherited volume boundaries",
            "Verify legacy production operations fixtures",
        }
        fixture_steps = [
            step for step in policy_steps if step.get("name") in independent_fixtures
        ]
        self.assertEqual({step["name"] for step in fixture_steps}, independent_fixtures)
        for step in fixture_steps:
            self.assertEqual(step.get("if"), "${{ !cancelled() }}")
            self.assertNotIn("continue-on-error", step)
        transition_step = next(
            step for step in fixture_steps
            if step["name"] == "Verify installed backup version transition and recovery"
        )
        self.assertEqual(
            transition_step["run"],
            "sudo /usr/bin/python3 scripts/ops/production-backup-transition-self-test.py",
        )
        wholeflow = next(step for step in fixture_steps
                         if step["name"] == "Verify whole-flow fixture and driver boundaries")
        self.assertEqual(wholeflow["run"], "sudo /usr/bin/python3 scripts/ops/owner-beta-daily-wholeflow-fixture-self-test.py")
        lifecycle = next(step for step in policy_steps
                         if step.get("name") == "Verify whole-flow bounded lifecycle recording")
        self.assertEqual(lifecycle["run"], "python3 scripts/ops/owner-beta-daily-lifecycle-self-test.py")
        self.assertEqual(lifecycle.get("if"), "${{ !cancelled() }}")
        self.assertNotIn("continue-on-error", lifecycle)
        lifecycle_docker = next(step for step in policy_steps
                                if step.get("name") == "Verify whole-flow actual Docker lifecycle events")
        self.assertEqual(lifecycle_docker["run"], "sudo /usr/bin/python3 scripts/ops/owner-beta-daily-lifecycle-docker-self-test.py")
        self.assertEqual(lifecycle_docker.get("if"), "${{ !cancelled() }}")
        self.assertNotIn("continue-on-error", lifecycle_docker)
        for name, script in (
            ("Verify pre-upgrade cold archive boundaries", "production-preupgrade-recovery-self-test.py"),
            ("Verify pre-upgrade maintenance failure boundaries", "production-preupgrade-maintenance-self-test.py"),
            ("Verify pre-upgrade schema59 isolated recovery", "production-preupgrade-recovery-docker-self-test.py"),
        ):
            self.assertEqual(
                next(step["run"] for step in fixture_steps if step["name"] == name),
                "sudo /usr/bin/python3 scripts/ops/" + script,
            )
        setup_node = next(
            step
            for step in policy_steps
            if step.get("uses", "").startswith("actions/setup-node@")
        )
        self.assertEqual(setup_node.get("with", {}).get("node-version"), "24")
        self.assertIn(
            "python -m pip install --disable-pip-version-check --no-cache-dir "
            "pyarrow==25.0.0 uv==0.12.1",
            text,
        )
        workspace_commands = command_lines(ci["jobs"]["workspace-tests"])
        self.assertIn(
            "cargo test -p api-server --test http_owner_equity_v2_chart --locked",
            workspace_commands,
        )
        self.assertIn("uv sync --project nt --locked", workspace_commands)
        self.assertLess(
            workspace_commands.index("uv sync --project nt --locked"),
            workspace_commands.index(workspace_test_command),
        )
        self.assertEqual(
            workspace_commands.count(workspace_test_command),
            1,
            "the full workspace test gate must explicitly enable test-support",
        )
        intraday_harness = (
            ROOT / "crates/job-queue/tests/intraday_quotes_support/mod.rs"
        ).read_text(encoding="utf-8")
        qa_urls = re.findall(
            r'pub const QA_DATABASE_URL: &str = "([^"]+)";', intraday_harness
        )
        self.assertEqual(len(qa_urls), 1)
        workspace_env = ci["jobs"]["workspace-tests"]["env"]
        self.assertEqual(workspace_env["DATABASE_URL"], qa_urls[0])
        self.assertEqual(
            workspace_env["LAGRANGE_QA_DB_PORT"], str(urlsplit(qa_urls[0]).port)
        )
        postgres_job = ci["jobs"]["postgres-integration-validation"]
        self.assertEqual(postgres_job["timeout-minutes"], "60")
        postgres_steps = postgres_job["steps"]
        self.assertTrue(any(step.get("if") == "always()" for step in postgres_steps))
        self.assertTrue(any(step.get("if") == "failure()" for step in postgres_steps))
        postgres_commands = "\n".join(step.get("run", "") for step in postgres_steps)
        self.assertIn("evidence.json", postgres_commands)
        self.assertIn("tail -n 200", postgres_commands)

        required = ci["jobs"]["required"]
        self.assertEqual(
            set(required["needs"]),
            {
                "policy",
                "format",
                "clippy",
                "web",
                "workspace-tests",
                "python-tests",
                "postgres-integration-validation",
                R4_VERIFICATION_JOB,
            },
        )
        self.assertEqual(
            required["steps"][0]["env"]["POSTGRES_VALIDATION_RESULT"],
            "${{ needs.postgres-integration-validation.result }}",
        )
        self.assertEqual(
            required["steps"][0]["env"]["WEB_RESULT"],
            "${{ needs.web.result }}",
        )
        self.assertEqual(
            required["steps"][0]["env"]["R4_VERIFICATION_BUILD_SELECTED"],
            R4_TRIGGER,
        )
        self.assertEqual(
            required["steps"][0]["env"]["R4_VERIFICATION_BUILD_RESULT"],
            "${{ needs.r4-verification-build.result }}",
        )
        qa_compose = (ROOT / "deploy" / "qa" / "qa-db.compose.yml").read_text(
            encoding="utf-8"
        )
        self.assertRegex(qa_compose, r"image:\s+postgres@sha256:[0-9a-f]{64}")
        self.assertIn(
            "127.0.0.1:${LAGRANGE_QA_DB_PORT:-55432}:5432", qa_compose
        )

    def test_required_validates_selected_and_unselected_r4_job_results(self) -> None:
        _, ci = load_workflow("ci.yml")
        required = ci["jobs"]["required"]
        self.assertEqual(required.get("if"), "always()")
        run_script = required["steps"][0]["run"]
        base_env = {
            "PATH": os.defpath,
            "POLICY_RESULT": "success",
            "FORMAT_RESULT": "success",
            "CLIPPY_RESULT": "success",
            "WEB_RESULT": "success",
            "TEST_RESULT": "success",
            "PYTHON_TEST_RESULT": "success",
            "POSTGRES_VALIDATION_RESULT": "success",
        }
        cases = (
            ("true", "success", 0),
            ("false", "skipped", 0),
            ("true", "skipped", 1),
            ("true", "failure", 1),
            ("false", "success", 1),
            ("false", "failure", 1),
        )
        for selected, result, expected_code in cases:
            env = {
                **base_env,
                "R4_VERIFICATION_BUILD_SELECTED": selected,
                "R4_VERIFICATION_BUILD_RESULT": result,
            }
            with self.subTest(selected=selected, result=result):
                completed = subprocess.run(
                    ["bash", "--noprofile", "--norc", "-e", "-c", run_script],
                    capture_output=True,
                    text=True,
                    env=env,
                    timeout=5,
                    check=False,
                )
                self.assertEqual(completed.returncode, expected_code, completed.stderr)

    def test_verify_all_gates_have_ci_counterparts(self) -> None:
        # Every hard gate declared by scripts/verify-all.sh ("Every gate is a
        # hard gate") must have a counterpart somewhere in the CI workflows.
        # verify-all.sh itself is not wired into any workflow (running it
        # serially would re-run fmt/clippy/cargo-test and blow past CI's
        # per-job timeouts), so CI re-implements its gates piecemeal. That
        # split invites silent drift: a gate can be removed from ci.yml
        # without anyone noticing verify-all.sh still promises it.
        #
        # Each row pairs a marker unique to one verify-all.sh gate with the
        # CI text that provides equivalent coverage. If a CI step providing
        # that coverage is deleted, the corresponding assertIn fails.
        verify_all_path = ROOT / "scripts" / "verify-all.sh"
        verify_all = verify_all_path.read_text(encoding="utf-8")
        ci_path, _ = load_workflow("ci.yml")
        ci_text = ci_path.read_text(encoding="utf-8")

        gates = [
            ('bash "$root/scripts/check-pins.sh"', "bash scripts/check-pins.sh"),
            (
                "for lf in Cargo.lock package-lock.json; do",
                "for lf in Cargo.lock package-lock.json; do",
            ),
            (
                'bash "$root/scripts/validate-foundation.sh"',
                "bash scripts/validate-foundation.sh",
            ),
            ("cargo fmt --all -- --check", "cargo fmt --all -- --check"),
            (
                "cargo clippy --workspace --all-targets --all-features --locked -- -D warnings",
                "cargo clippy --workspace --all-targets --all-features --locked -- -D warnings",
            ),
            (
                "cargo test --workspace --locked --no-fail-fast --features kis-client/test-support",
                "cargo test --workspace --locked --no-fail-fast --features kis-client/test-support",
            ),
            (
                "npm run lint --workspaces --if-present",
                "npm run lint --workspace @lagrange/web",
            ),
            (
                "npm run typecheck --workspaces --if-present",
                "npm run typecheck --workspace @lagrange/web",
            ),
            ("npm test --workspaces --if-present", "npm test --workspace @lagrange/web"),
            ("uv run --project nt pytest -q", "uv run --project nt pytest -q"),
            (
                'bash "$root/scripts/backup/tests/test-validate-policy.sh"',
                "bash scripts/backup/tests/test-validate-policy.sh",
            ),
        ]

        # If a gate is added to (or removed from) verify-all.sh without a
        # matching update to the table above, the step count diverges and
        # this catches it before the per-gate assertions even run.
        step_count = len(re.findall(r'echo "\[\$step\]', verify_all))
        self.assertEqual(
            step_count,
            len(gates),
            "scripts/verify-all.sh gate count changed; update the gate/CI "
            "mapping in test_verify_all_gates_have_ci_counterparts",
        )

        for verify_marker, ci_marker in gates:
            self.assertIn(
                verify_marker,
                verify_all,
                f"gate marker no longer found in scripts/verify-all.sh: {verify_marker!r}",
            )
            self.assertIn(
                ci_marker,
                ci_text,
                "scripts/verify-all.sh gate has no CI counterpart "
                f"(missing from ci.yml): {ci_marker!r}",
            )

        powershell_path = ROOT / "scripts" / "verify-all.ps1"
        powershell_text = powershell_path.read_text(encoding="utf-8")
        rust_commands = (
            "cargo fmt --all -- --check",
            "cargo clippy --workspace --all-targets --all-features --locked -- -D warnings",
            "cargo test --workspace --locked --no-fail-fast --features kis-client/test-support",
        )
        for command in rust_commands:
            self.assertIn(command, verify_all, f"Rust command missing from verify-all.sh: {command}")
            self.assertIn(command, powershell_text, f"Rust command missing from verify-all.ps1: {command}")

    def test_smoke_runs_only_the_existing_functional_script(self) -> None:
        path, smoke = load_workflow("research-smoke.yml")
        self.assertEqual(set(smoke["jobs"]), {"research-smoke"})
        text = path.read_text(encoding="utf-8")
        self.assertIn("bash scripts/qa/research-worker-smoke.sh", text)
        self.assertNotIn("--static-only", text)

        script = (ROOT / "scripts" / "qa" / "research-worker-smoke.sh").read_text(
            encoding="utf-8"
        )
        for fixture in (
            'RANGE_RAW_BATCH_ID="${RANGE_RAW_BATCH_ID:-00000000-0000-0000-0000-000000000001}"',
            'BACKTEST_MIN_FREE_BYTES="${BACKTEST_MIN_FREE_BYTES:-1073741824}"',
            'BACKTEST_MAX_QUEUED_BACKTESTS="${BACKTEST_MAX_QUEUED_BACKTESTS:-1000}"',
            'BACKTEST_RECONCILE_GRACE_SECS="${BACKTEST_RECONCILE_GRACE_SECS:-900}"',
            'BACKTEST_RECONCILE_INTERVAL_SECS="${BACKTEST_RECONCILE_INTERVAL_SECS:-60}"',
        ):
            self.assertIn(fixture, script)
        powershell_script = (
            ROOT / "scripts" / "qa" / "research-worker-smoke.ps1"
        ).read_text(encoding="utf-8")
        self.assertIn(
            "$env:RANGE_RAW_BATCH_ID = '00000000-0000-0000-0000-000000000001'",
            powershell_script,
        )
        self.assertIn("provision-runtime-secrets.sh|runtime-static-check.sh", script)
        self.assertNotIn("README.md|.gitignore|*.example|*.sh", script)
        self.assertEqual(
            re.findall(r'^build_service_image ([^\n\\]+)', script, re.MULTILINE),
            ["research-worker ", "candidate-runner "],
        )
        self.assertIn("export COMPOSE_PARALLEL_LIMIT=1", script)
        self.assertIn("export CARGO_BUILD_JOBS=2", script)
        self.assertIn("candidate-runner --once", script)
        self.assertIn("candidate_feed_evidence", script)
        self.assertIn(
            '["dataset","recommendation","candidate","backtest","paper_view"]',
            script,
        )
        self.assertIn("manual_reference_evidence", script)
        self.assertIn("public.register_candidate_instrument(", script)
        self.assertIn("candidate_instrument_registration_state", script)
        self.assertLess(
            script.index("candidate_instrument_registration_state="),
            script.index("candidate_one_shot()"),
        )
        self.assertIn(
            "2|2|kospi200,kosdaq150|2|10|kospi200:5,kosdaq150:5",
            script,
        )
        function = script.split("schema_gate_must_pass() {", 1)[1].split("}", 1)[0]
        self.assertIn("schema_output=", function)
        self.assertNotIn(">/dev/null 2>&1", function)

        secret_setup = script.split("umask 077", 1)[1].split(
            "export LAGRANGE_RUNTIME_SECRET_DIR", 1
        )[0]
        self.assertIn(
            'find "$runtime_secret_root" -type f -exec chmod 0444 {} +',
            secret_setup,
        )
        self.assertIn("tr -d '\\r\\n'", secret_setup)
        self.assertIn(
            'schema_postgres_secret="$runtime_secret_root/research-schema-check/postgres_password"',
            script,
        )
        self.assertIn("ledger_state=", script)
        self.assertIn('if [ "$ledger_state" != "11" ]', script)
        self.assertIn("candidate_source_evidence()", script)
        self.assertIn("krx_kosdaq150_membership", script)
        self.assertIn("candidate_before=", script)
        self.assertIn("candidate_after=", script)
        self.assertIn("rc up -d --no-deps research-worker", script)

        migration_loop = script.split("while IFS= read -r migration; do", 1)[1].split(
            "done < <(", 1
        )[0]
        ledger_insert = next(
            line
            for line in migration_loop.splitlines()
            if '-c "INSERT INTO _sqlx_migrations' in line
        )
        self.assertIn("</dev/null", ledger_insert)

        for smoke_name in ("research-worker-smoke.sh", "research-worker-smoke.ps1"):
            smoke_text = (ROOT / "scripts" / "qa" / smoke_name).read_text(
                encoding="utf-8"
            )
            self.assertNotIn("provider=KRX/market=KR", smoke_text)
            self.assertIn("provider=krx/market=kr", smoke_text)
            self.assertNotIn("bool_and(c.source_batch_id = source.id)", smoke_text)
            self.assertIn("c.source_batch_id IS NOT NULL", smoke_text)
            self.assertIn("batch.source_batch_id = c.source_batch_id", smoke_text)
            self.assertIn("history.content_sha256 = c.content_sha256", smoke_text)
            if smoke_name.endswith(".sh"):
                self.assertIn(
                    'find /data/raw -xdev -path "$calendar_bootstrap" -prune -o -type d',
                    smoke_text,
                )
                self.assertIn(
                    'find /data/raw -xdev -path "$calendar_bootstrap" -prune -o -type f',
                    smoke_text,
                )
                self.assertIn("find /data/curated -xdev -type d", smoke_text)
                self.assertIn("find /data/curated -xdev -type f", smoke_text)
                self.assertIn(
                    "calendar_bootstrap=/data/raw/.calendar-bootstrap", smoke_text
                )
                self.assertIn(
                    'init_command.count("validate_calendar_bootstrap") >= 3',
                    smoke_text,
                )
                self.assertIn("10001:10001:700", smoke_text)
                self.assertIn("10001:10001:600:1", smoke_text)
                for contract in (
                    "chown 10001:10001",
                    "chmod 0750",
                    "chmod 0640",
                    "chmod 0440",
                    "validator accepted an unpruned Raw init",
                    "validator accepted a symlink-following Raw init",
                    'sed \'s# -path "\\$\\$calendar_bootstrap" -prune -o##g\'',
                    'cmp -- "$calendar_before" "$calendar_after"',
                    "Raw init changed valid calendar metadata or bytes",
                    "idempotent Raw init changed valid calendar metadata or bytes",
                    "wrong directory mode",
                    "wrong claim mode",
                    "wrong directory owner",
                    "wrong claim owner",
                    "claim symlink",
                    "claim hardlink",
                    "unknown direct entry",
                    "nested direct entry",
                ):
                    self.assertIn(contract, smoke_text)
            else:
                # This is the legacy PowerShell implementation. Keep its
                # current literals checked without asserting calendar parity.
                self.assertIn("find /data/raw -xdev -type d", smoke_text)
                self.assertIn("find /data/raw -xdev -type f", smoke_text)
                self.assertIn("find -L /data/raw -type l", smoke_text)
                self.assertIn(
                    "validator accepted a symlink-following Raw init", smoke_text
                )

    def test_candidate_static_check_uses_scoped_entitlement_fixture(self) -> None:
        script = (
            ROOT / "deploy" / "compose" / "candidate-static-check.sh"
        ).read_text(encoding="utf-8")
        compose = (ROOT / "deploy" / "compose" / "compose.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn('env_file="$root/deploy/compose/.env.example"', script)
        self.assertIn('docker compose --env-file "$env_file" -f "$compose" config --quiet', script)
        self.assertIn(
            "RESEARCH_ENTITLEMENT_SHA256=0000000000000000000000000000000000000000000000000000000000000000 \\",
            script,
        )
        self.assertIn(
            "${RESEARCH_ENTITLEMENT_SHA256:?RESEARCH_ENTITLEMENT_SHA256 must identify the exact active data-use contract hash}",
            compose,
        )

    def test_calendar_lock_probe_is_a_standalone_locked_qa_fixture(self) -> None:
        fixture = ROOT / "scripts" / "qa" / "calendar-lock-probe"
        manifest = (fixture / "Cargo.toml").read_text(encoding="utf-8")
        lockfile = (fixture / "Cargo.lock").read_text(encoding="utf-8")
        source = (fixture / "src" / "main.rs").read_text(encoding="utf-8")
        script = (ROOT / "scripts" / "qa" / "research-worker-smoke.sh").read_text(
            encoding="utf-8"
        )
        self.assertIn("[workspace]", manifest)
        self.assertIn('name = "calendar-lock-probe"', manifest)
        self.assertIn('name = "calendar-lock-probe"', lockfile)
        self.assertIn('domain = { path = "../../../crates/domain" }', manifest)
        self.assertIn(
            '#[path = "../../../../data-pipelines/collectors/src/calendar_claim.rs"]',
            source,
        )
        self.assertIn("CalendarDayLock::acquire", source)
        self.assertIn("cargo fetch --locked --manifest-path", script)
        self.assertIn("cargo build --locked --offline", script)
        self.assertIn(
            "--manifest-path /source/scripts/qa/calendar-lock-probe/Cargo.toml",
            script,
        )
        self.assertNotIn("cargo generate-lockfile", script)
        self.assertNotIn("/home/l1nnx/.cargo/registry", script)
        self.assertIn('cargo_home_volume="${probe_id}-cargo-home"', script)
        self.assertIn('cargo_target_volume="${probe_id}-cargo-target"', script)
        prep = script.split("# This preparation container", 1)[1].split(
            "calendar probe locked dependency preparation failed", 1
        )[0]
        self.assertIn("--network bridge", prep)
        self.assertIn(':/source:ro', prep)
        self.assertNotIn("/data/raw", prep)
        self.assertNotIn("/data/curated", prep)
        self.assertIn("CARGO_BUILD_JOBS=2 cargo fetch --locked --manifest-path Cargo.toml", prep)

        root_lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
        fixture_lock = tomllib.loads(lockfile)
        root_registry_packages = {
            (package["name"], package["version"], package.get("source"))
            for package in root_lock["package"]
            if package.get("source", "").startswith("registry+")
        }
        fixture_registry_packages = {
            (package["name"], package["version"], package.get("source"))
            for package in fixture_lock["package"]
            if package.get("source", "").startswith("registry+")
        }
        self.assertFalse(fixture_registry_packages - root_registry_packages)

    def test_candidate_entitlement_hash_remains_required_in_production_compose(self) -> None:
        compose = (ROOT / "deploy" / "compose" / "compose.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("RESEARCH_ENTITLEMENT_SHA256: ${RESEARCH_ENTITLEMENT_SHA256:?", compose)


class ResearchSmokeCleanupTests(unittest.TestCase):
    def run_cleanup(
        self, original_exit: int = 0, failure: str = "", probe: bool = False
    ) -> subprocess.CompletedProcess[str]:
        source = (ROOT / "scripts/qa/research-worker-smoke.sh").read_text("utf-8")
        functions = source.split("cleanup_command() {", 1)[1].split(
            "trap cleanup EXIT", 1
        )[0]
        functions = "cleanup_command() {" + functions
        if probe:
            functions += "probe_cleanup() {" + source.split(
                "probe_cleanup() {", 1
            )[1].split("  trap probe_cleanup EXIT", 1)[0]
        stubs = r'''
rc() {
  printf '%s\n' 'PRIVATE_DIAGNOSTIC_SENTINEL' >&2
  if [ "$1" = "$FAILURE" ]; then return 17; fi
}
dkr() {
  printf '%s\n' 'PRIVATE_DIAGNOSTIC_SENTINEL' >&2
  if [ "$1 $2" = 'ps -aq' ]; then
    if [ "$FAILURE" = query ]; then return 19; fi
    if [ "$FAILURE" = remaining ]; then printf '%s\n' 'PRIVATE_RESOURCE_SENTINEL'; fi
  fi
  if [ "$1 $2" = 'volume rm' ] && [ "$FAILURE" = probe ]; then return 23; fi
  return 0
}
created=1
context_audit_created=1
smoke_started=0 build_seconds=0 host_cargo_seconds=0
research_worker_cache_promoted=false candidate_runner_cache_promoted=false
research_worker_cache_bytes=0 candidate_runner_cache_bytes=0
GITHUB_OUTPUT=/dev/null
project=isolated-cleanup-test
context_audit_tag=isolated-cleanup-test-context
raw_volume=a curated_volume=b outside_volume=c binary_volume=d cargo_home_volume=e cargo_target_volume=f
'''
        with tempfile.TemporaryDirectory() as directory:
            script = "set -euo pipefail\n" + functions + stubs
            script += f"trap {'probe_cleanup' if probe else 'cleanup'} EXIT\nexit {original_exit}\n"
            return subprocess.run(
                ["bash", "-c", script],
                env={"PATH": os.defpath, "FAILURE": failure, "temp_root": directory},
                text=True,
                capture_output=True,
                timeout=10,
                check=False,
            )

    def test_success_records_absence_and_every_cleanup_stage(self) -> None:
        result = self.run_cleanup()
        self.assertEqual(result.returncode, 0, result.stderr)
        for step in (
            "stop-research", "empty-fixture-data", "compose-down",
            "project-containers", "project-volumes", "project-networks",
            "context-image", "context-image-absence", "temporary-directory",
        ):
            self.assertIn(f"cleanup step={step} exit=0", result.stdout)
        self.assertIn("original_exit=0 cleanup_exit=0", result.stdout)
        self.assertNotIn("PRIVATE_", result.stdout + result.stderr)

    def test_cleanup_failure_fails_an_otherwise_successful_smoke(self) -> None:
        result = self.run_cleanup(failure="down")
        self.assertEqual(result.returncode, 1)
        self.assertIn("step=compose-down exit=17", result.stdout)
        self.assertIn("step=temporary-directory exit=0", result.stdout)

    def test_cleanup_failure_preserves_the_original_failure(self) -> None:
        result = self.run_cleanup(original_exit=7, failure="down")
        self.assertEqual(result.returncode, 7)
        self.assertIn("original_exit=7 cleanup_exit=1", result.stdout)

    def test_failed_absence_query_is_unknown_and_fails_closed(self) -> None:
        result = self.run_cleanup(failure="query")
        self.assertEqual(result.returncode, 1)
        self.assertIn("step=project-containers exit=19 remaining=UNKNOWN", result.stdout)
        self.assertNotIn("PRIVATE_", result.stdout + result.stderr)

    def test_leftover_resources_fail_without_printing_names(self) -> None:
        result = self.run_cleanup(failure="remaining")
        self.assertEqual(result.returncode, 1)
        self.assertIn("step=project-containers exit=0 remaining=PRESENT", result.stdout)
        self.assertNotIn("PRIVATE_", result.stdout + result.stderr)

    def test_probe_removal_failure_fails_an_otherwise_successful_probe(self) -> None:
        result = self.run_cleanup(failure="probe", probe=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn("step=raw-probe-volumes exit=23", result.stdout)

    def test_probe_removal_failure_preserves_the_original_failure(self) -> None:
        result = self.run_cleanup(original_exit=7, failure="probe", probe=True)
        self.assertEqual(result.returncode, 7)
        self.assertIn("raw-probe cleanup original_exit=7 cleanup_exit=1", result.stdout)


if __name__ == "__main__":
    unittest.main()
