from __future__ import annotations

import hashlib
import copy
import json
import os
import re
import shlex
import subprocess
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
            self.assertNotIn("actions/upload-artifact", text)
            self.assertNotIn("target/", text)
            self.assertNotIn("pull_request_target", text)
            self.assertNotIn("secrets.", text)
            self.assertNotRegex(text, r"steps\.[a-z-]+-save\.outputs")
            assert_cache_policy(workflow)

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
        web_commands = [line for line in command_lines(ci["jobs"]["web"]) if line in expected_web_commands]
        self.assertEqual(
            web_commands,
            [
                "npm ci",
                "npx playwright install --with-deps chromium",
                "npm run openapi:check --workspace @lagrange/api-server",
                "npm run lint --workspace @lagrange/web",
                "npm run typecheck --workspace @lagrange/web",
                "npm test --workspace @lagrange/web",
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
        qa_compose = (ROOT / "deploy" / "qa" / "qa-db.compose.yml").read_text(
            encoding="utf-8"
        )
        self.assertRegex(qa_compose, r"image:\s+postgres@sha256:[0-9a-f]{64}")
        self.assertIn(
            "127.0.0.1:${LAGRANGE_QA_DB_PORT:-55432}:5432", qa_compose
        )

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


if __name__ == "__main__":
    unittest.main()
