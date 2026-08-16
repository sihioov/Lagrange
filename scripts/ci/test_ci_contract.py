from __future__ import annotations

import hashlib
import json
import re
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".github" / "workflows"
SHA_ACTION = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")


def load_workflow(name: str) -> tuple[Path, dict]:
    path = WORKFLOWS / name
    if not path.is_file():
        raise AssertionError(f"missing workflow: {path.relative_to(ROOT)}")
    parsed = yaml.load(path.read_text(encoding="utf-8"), Loader=yaml.BaseLoader)
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


class WorkflowContractTests(unittest.TestCase):
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
            for job in workflow["jobs"].values():
                self.assertEqual(job["runs-on"], "ubuntu-latest")
                self.assertIn("timeout-minutes", job)
                for step in job.get("steps", []):
                    if step.get("uses", "").startswith("actions/checkout@"):
                        self.assertEqual(
                            step.get("with", {}).get("persist-credentials"), "false"
                        )
            text = path.read_text(encoding="utf-8").lower()
            self.assertNotIn("actions/upload-artifact", text)
            self.assertNotIn("actions/cache", text)
            self.assertNotIn("target/", text)

    def test_ci_runs_required_gates_and_disposable_data(self) -> None:
        path, ci = load_workflow("ci.yml")
        self.assertEqual(
            set(ci["jobs"]),
            {
                "policy",
                "format",
                "clippy",
                "workspace-tests",
                "postgres-integration-validation",
                "required",
            },
        )
        text = path.read_text(encoding="utf-8")
        self.assertIn("scripts/ci/prepare_phase0.py --root data/phase0", text)
        self.assertIn("deploy/qa/qa-db.compose.yml up -d --wait", text)
        self.assertIn("cargo test --workspace --locked --no-fail-fast", text)
        self.assertIn(
            "bash deploy/db/integration-validation/static-check.sh", text
        )
        self.assertIn(
            "bash deploy/db/integration-validation/validate.sh --self-test", text
        )
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
        policy_steps = ci["jobs"]["policy"]["steps"]
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
                "workspace-tests",
                "postgres-integration-validation",
            },
        )
        self.assertEqual(
            required["steps"][0]["env"]["POSTGRES_VALIDATION_RESULT"],
            "${{ needs.postgres-integration-validation.result }}",
        )
        qa_compose = (ROOT / "deploy" / "qa" / "qa-db.compose.yml").read_text(
            encoding="utf-8"
        )
        self.assertRegex(qa_compose, r"image:\s+postgres@sha256:[0-9a-f]{64}")

    def test_smoke_runs_only_the_existing_functional_script(self) -> None:
        path, smoke = load_workflow("research-smoke.yml")
        self.assertEqual(set(smoke["jobs"]), {"research-smoke"})
        text = path.read_text(encoding="utf-8")
        self.assertIn("bash scripts/qa/research-worker-smoke.sh", text)
        self.assertNotIn("--static-only", text)

        script = (ROOT / "scripts" / "qa" / "research-worker-smoke.sh").read_text(
            encoding="utf-8"
        )
        function = script.split("schema_gate_must_pass() {", 1)[1].split("}", 1)[0]
        self.assertIn("schema_output=", function)
        self.assertNotIn(">/dev/null 2>&1", function)

        secret_setup = script.split("umask 077", 1)[1].split(
            "export LAGRANGE_POSTGRES_PASSWORD_SECRET_SOURCE", 1
        )[0]
        self.assertIn(
            'chmod 0444 "$postgres_secret" "$research_secret" "$krx_secret"',
            secret_setup,
        )

        self.assertIn("ledger_state=", script)
        self.assertIn('if [ "$ledger_state" != "7" ]', script)

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
            self.assertIn("find /data/raw -mindepth 1 -delete", smoke_text)


if __name__ == "__main__":
    unittest.main()
