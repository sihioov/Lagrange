from __future__ import annotations

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
            {"policy", "format", "clippy", "workspace-tests", "required"},
        )
        text = path.read_text(encoding="utf-8")
        self.assertIn("scripts/ci/prepare_phase0.py --root data/phase0", text)
        self.assertIn("deploy/qa/qa-db.compose.yml up -d --wait", text)
        self.assertIn("cargo test --workspace --locked --no-fail-fast", text)
        self.assertIn(
            "cargo clippy --workspace --all-targets --all-features --locked -- -D warnings",
            text,
        )
        self.assertIn("cargo fmt --all -- --check", text)
        self.assertIn("CARGO_INCREMENTAL: '0'", text)
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


if __name__ == "__main__":
    unittest.main()
