from __future__ import annotations

import contextlib
import copy
import errno
import hashlib
import importlib.util
import io
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "qa" / "dev-check.py"
SPEC = importlib.util.spec_from_file_location("dev_check_under_test", SCRIPT)
assert SPEC and SPEC.loader
dev_check = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = dev_check
SPEC.loader.exec_module(dev_check)


def command(args: list[str], cwd: Path, *, env: dict[str, str] | None = None) -> str:
    result = subprocess.run(
        args,
        cwd=cwd,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode:
        raise AssertionError(f"command failed ({result.returncode}): {args[0]}")
    return result.stdout.strip()


class TemporaryRepository:
    def __init__(self, check: dict | None = None, *, tools: dict | None = None) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="dev-check-test-")
        self.root = Path(self.temp.name)
        (self.root / "scripts" / "qa").mkdir(parents=True)
        (self.root / "src").mkdir()
        shutil.copyfile(SCRIPT, self.root / "scripts" / "qa" / "dev-check.py")
        (self.root / "src" / "input.py").write_text("value = 1\n", encoding="utf-8")
        (self.root / "Cargo.lock").write_text("lock-version = 1\n", encoding="utf-8")
        (self.root / "settings.json").write_text("{\"mode\": \"test\"}\n", encoding="utf-8")
        (self.root / "docs").mkdir()
        (self.root / "docs" / "note.md").write_text("baseline\n", encoding="utf-8")
        (self.root / "target").mkdir(mode=0o700)
        os.chmod(self.root / "target", 0o700)
        command(["git", "init", "-q"], self.root)
        command(["git", "config", "user.email", "dev-check@example.invalid"], self.root)
        command(["git", "config", "user.name", "dev-check-test"], self.root)
        self.config = {
            "schema_version": 1,
            "dispatcher_version": "test",
            "common_patterns": [],
            "path_rules": [],
            "tools": tools or {
                "git": copy.deepcopy(dev_check.load_config(ROOT)["tools"]["git"]),
                "python3": copy.deepcopy(dev_check.load_config(ROOT)["tools"]["python3"]),
            },
            "profiles": {
                "qa": {"description": "test", "fast": ["python-syntax"], "focused": [], "integration": []},
                "rust": {"description": "test", "fast": [], "focused": ["python-syntax"], "integration": []},
                "all": {"description": "test", "fast": [], "focused": [], "integration": []},
            },
            "checks": {
                "python-syntax": check or {
                    "label": "Python syntax fake",
                    "argv": [sys.executable, "-B", "-c", "import ast,pathlib,sys; ast.parse(pathlib.Path(sys.argv[1]).read_text())", "src/input.py"],
                    "cwd": ".",
                    "inputs": ["src/**/*.py", "Cargo.lock", "settings.json"],
                    "reuse": True,
                    "reuse_kind": "python-syntax",
                    "tools": ["python3"],
                    "fingerprint_env": ["DEV_CHECK_TEST_ENV"],
                    "timeout_seconds": 20,
                },
            },
            "integration_requirements": {
                "qa": {"tools": [], "paths": [], "required_env": [], "ports": [], "docker_daemon": False},
                "rust": {"tools": [], "paths": [], "required_env": [], "ports": [], "docker_daemon": False},
                "all": {"tools": [], "paths": [], "required_env": [], "ports": [], "docker_daemon": False},
            },
            "integration_plans": {"qa": [], "all": []},
        }
        self.write_config()
        command(["git", "add", "."], self.root)
        command(["git", "commit", "-qm", "baseline"], self.root)

    def write_config(self) -> None:
        (self.root / "scripts" / "qa" / "check-profiles.json").write_text(
            json.dumps(self.config, sort_keys=True, indent=2) + "\n", encoding="utf-8"
        )

    def close(self) -> None:
        self.temp.cleanup()


class DevCheckCliTests(unittest.TestCase):
    def test_help_states_defaults_and_root_independent_profile_selection(self) -> None:
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "preflight", "--help"],
            cwd=ROOT / "apps" / "web",
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("default: HEAD", result.stdout)
        self.assertIn("default: local", result.stdout)
        self.assertIn("independent of the caller's current directory", result.stdout)

    def test_plan_is_same_from_repository_root_and_web_subdirectory(self) -> None:
        outputs: list[dict] = []
        for cwd in (ROOT, ROOT / "apps" / "web"):
            result = subprocess.run(
                [sys.executable, str(SCRIPT), "plan", "--base", "HEAD", "--format", "json"],
                cwd=cwd,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            outputs.append(json.loads(result.stdout))
        self.assertEqual(outputs[0]["changed_paths"], outputs[1]["changed_paths"])
        self.assertEqual(outputs[0]["profiles"], outputs[1]["profiles"])
        for profile in outputs[0]["checks"].values():
            for stage in profile.values():
                for check in stage:
                    self.assertEqual(check["cwd"], str(ROOT))

    def test_explicit_run_from_root_and_child_uses_same_profile_stage(self) -> None:
        repo = TemporaryRepository()
        try:
            child = repo.root / "apps" / "web"
            child.mkdir(parents=True)
            script = repo.root / "scripts" / "qa" / "dev-check.py"
            args = [sys.executable, str(script), "run", "--profile", "qa", "--stage", "fast"]
            first = subprocess.run(
                args,
                cwd=repo.root,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            second = subprocess.run(
                [*args, "--reuse"],
                cwd=child,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            self.assertEqual(first.returncode, 0, first.stderr)
            self.assertEqual(second.returncode, 0, second.stderr)
            self.assertIn("PASS: python-syntax", first.stdout)
            self.assertIn("REUSED: python-syntax", second.stdout)
        finally:
            repo.close()

    def test_focused_run_stops_when_prerequisite_fast_stage_fails(self) -> None:
        repo = TemporaryRepository()
        try:
            sentinel = repo.root / "focused-started"
            repo.config["profiles"]["qa"]["fast"] = ["fast-failure"]
            repo.config["profiles"]["qa"]["focused"] = ["expensive-child"]
            repo.config["checks"]["fast-failure"] = {
                "label": "fast prerequisite failure",
                "argv": [sys.executable, "-B", "-c", "raise SystemExit(7)"],
                "cwd": ".",
                "inputs": ["src/**/*.py"],
                "reuse": False,
                "tools": ["python3"],
                "timeout_seconds": 10,
            }
            repo.config["checks"]["expensive-child"] = {
                "label": "must not start",
                "argv": [sys.executable, "-B", "-c", "import pathlib,sys; pathlib.Path(sys.argv[1]).write_text('started')", str(sentinel)],
                "cwd": ".",
                "inputs": ["src/**/*.py"],
                "reuse": False,
                "tools": ["python3"],
                "timeout_seconds": 10,
            }
            repo.write_config()
            script = repo.root / "scripts" / "qa" / "dev-check.py"
            result = subprocess.run(
                [sys.executable, str(script), "run", "--profile", "qa", "--stage", "focused"],
                cwd=repo.root,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn("FAILED: fast-failure", result.stdout)
            self.assertIn("Focused checks were not started because the fast stage failed.", result.stdout)
            self.assertFalse(sentinel.exists())
        finally:
            repo.close()

    def test_focused_run_executes_fast_before_focused_checks(self) -> None:
        repo = TemporaryRepository()
        try:
            order = repo.root / "execution-order.txt"
            child = repo.root / "apps" / "web"
            child.mkdir(parents=True)
            repo.config["profiles"]["qa"]["fast"] = ["fast-step"]
            repo.config["profiles"]["qa"]["focused"] = ["focused-step"]
            for check_id, label, step in (
                ("fast-step", "fast prerequisite", "fast"),
                ("focused-step", "focused check", "focused"),
            ):
                repo.config["checks"][check_id] = {
                    "label": label,
                    "argv": [sys.executable, "-B", "-c", "import pathlib,sys; open(sys.argv[1],'a',encoding='utf-8').write(sys.argv[2]+'\\n')", str(order), step],
                    "cwd": ".",
                    "inputs": ["src/**/*.py"],
                    "reuse": False,
                    "tools": ["python3"],
                    "timeout_seconds": 10,
                }
            repo.write_config()
            script = repo.root / "scripts" / "qa" / "dev-check.py"
            result = subprocess.run(
                [sys.executable, str(script), "run", "--profile", "qa", "--stage", "focused"],
                cwd=child,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertLess(result.stdout.index("PASS: fast-step"), result.stdout.index("PASS: focused-step"))
            self.assertEqual(order.read_text(encoding="utf-8").splitlines(), ["fast", "focused"])
        finally:
            repo.close()

    def test_nonexistent_base_is_a_specific_input_error(self) -> None:
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "plan", "--base", "does-not-exist-xyz", "--format", "json"],
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("base ref does not resolve to a commit", result.stderr)

    def test_preflight_rejects_missing_base_ref_without_fetching(self) -> None:
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "preflight", "--profile", "qa", "--stage", "fast", "--base", "missing-preflight-ref-xyz"],
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("base ref does not resolve to a commit: missing-preflight-ref-xyz", result.stderr)

    def test_only_real_workspace_packages_are_accepted(self) -> None:
        self.assertEqual(len(dev_check.workspace_packages(ROOT)), 16)
        dev_check.validate_package("rust", "focused", "domain", ROOT)
        dev_check.validate_package("rust", "focused", "job-queue", ROOT)
        with self.assertRaisesRegex(dev_check.InputError, "not a Cargo workspace package"):
            dev_check.validate_package("rust", "focused", "not-a-member", ROOT)
        with self.assertRaisesRegex(dev_check.InputError, "supported only"):
            dev_check.validate_package("web", "focused", "domain", ROOT)

    def test_workspace_package_parser_ignores_toml_11_outside_package_table(self) -> None:
        with tempfile.TemporaryDirectory(prefix="dev-check-toml11-") as temp:
            root = Path(temp)
            package_dir = root / "crates" / "toml11-package"
            package_dir.mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["crates/toml11-package"]\n', encoding="utf-8"
            )
            manifest = package_dir / "Cargo.toml"
            source = (
                '[package]\nname = "toml11-package"\nversion = "0.1.0"\nedition = "2024"\n'
                '[dependencies]\nexample = {\n  version = "1",\n  features = ["fixture"],\n}\n'
            )
            manifest.write_text(source, encoding="utf-8")
            with self.assertRaises(dev_check.tomllib.TOMLDecodeError):
                dev_check.tomllib.loads(source)
            self.assertEqual(dev_check.workspace_packages(root), {"toml11-package"})

    def test_integration_run_only_prints_hosted_commands_and_prerequisites(self) -> None:
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "run", "--profile", "db", "--stage", "integration"],
            cwd=ROOT / "apps" / "web",
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("INTEGRATION PLAN ONLY", result.stdout)
        self.assertIn("bash deploy/db/integration-validation/validate.sh --evidence-dir", result.stdout)
        self.assertIn("ports 55432, 55433, and 55434", result.stdout)
        self.assertNotIn("integration passed", result.stdout.lower())


class GitDiscoveryTests(unittest.TestCase):
    def test_base_index_worktree_untracked_deletion_rename_and_secret_filtering(self) -> None:
        repo = TemporaryRepository()
        try:
            (repo.root / ".gitignore").write_text("*.ignored\n", encoding="utf-8")
            (repo.root / "rename-old.py").write_text("def stable_name():\n    return 'stable'\n", encoding="utf-8")
            (repo.root / "deleted.py").write_text("value = 'removed'\n", encoding="utf-8")
            (repo.root / "staged.py").write_text("value = 'before'\n", encoding="utf-8")
            (repo.root / "working.py").write_text("value = 'before'\n", encoding="utf-8")
            command(["git", "add", "."], repo.root)
            command(["git", "commit", "-qm", "discovery fixture"], repo.root)
            base = command(["git", "rev-parse", "HEAD"], repo.root)

            command(["git", "mv", "rename-old.py", "rename-new.py"], repo.root)
            command(["git", "rm", "deleted.py"], repo.root)
            command(["git", "commit", "-qm", "rename and delete"], repo.root)
            (repo.root / "staged.py").write_text("value = 'staged-change'\n", encoding="utf-8")
            command(["git", "add", "staged.py"], repo.root)
            (repo.root / "working.py").write_text("value = 'worktree-change'\n", encoding="utf-8")
            (repo.root / "scripts" / "qa" / "untracked.py").write_text("value = 3\n", encoding="utf-8")
            (repo.root / "ignored.ignored").write_text("ignore this content\n", encoding="utf-8")
            (repo.root / ".env.local").write_text("SENSITIVE_SENTINEL_CONTENT\n", encoding="utf-8")

            changes = dev_check.collect_changes(repo.root, base)
            paths = set(changes["changed_paths"])
            self.assertIn("rename-old.py", paths)
            self.assertIn("rename-new.py", paths)
            self.assertIn("deleted.py", paths)
            self.assertIn("staged.py", paths)
            self.assertIn("working.py", paths)
            self.assertIn("scripts/qa/untracked.py", paths)
            self.assertNotIn("ignored.ignored", paths)
            self.assertNotIn(".env.local", paths)
            self.assertEqual(changes["sensitive_changes_count"], 1)
            self.assertNotIn("SENSITIVE_SENTINEL_CONTENT", json.dumps(changes))
            self.assertIn("deleted.py", changes["all_paths"])
        finally:
            repo.close()

    def test_unknown_and_common_changes_widen_to_all(self) -> None:
        config = dev_check.load_config(ROOT)
        changes = {"changed_paths": ["outside/new_contract.bin"], "sensitive_changes_count": 0}
        selection = dev_check.select_profiles(config, changes)
        self.assertEqual(selection["profiles"], ["all"])
        self.assertEqual(selection["unclassified_paths"], ["outside/new_contract.bin"])
        self.assertTrue(selection["requires_full_integration"])
        common = dev_check.select_profiles(config, {"changed_paths": ["Cargo.lock"], "sensitive_changes_count": 0})
        self.assertEqual(common["profiles"], ["all"])
        self.assertTrue(common["widened_to_all"])

    def test_whitespace_check_covers_untracked_files(self) -> None:
        repo = TemporaryRepository()
        try:
            untracked = repo.root / "src" / "new.py"
            untracked.write_text("value = 2 \n", encoding="utf-8")
            changes = dev_check.collect_changes(repo.root, "HEAD")
            check = dev_check.load_config(ROOT)["checks"]["git-diff-check"]
            argv = dev_check.expand_argv("git-diff-check", check, changes, repo.root)
            result = subprocess.run(
                argv,
                cwd=repo.root,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("trailing whitespace", result.stdout)
        finally:
            repo.close()


class PreflightBoundaryTests(unittest.TestCase):
    def make_sentinel_repo(self, check: dict, *, tools: dict | None = None) -> TemporaryRepository:
        repo = TemporaryRepository(check, tools=tools)
        return repo

    def run_and_capture(self, repo: TemporaryRepository, *, env: dict[str, str] | None = None) -> tuple[int, dict]:
        with contextlib.redirect_stdout(io.StringIO()):
            status, report = dev_check.run_profile(
                repo.root,
                repo.config,
                "qa",
                "fast",
                environment="local",
                reuse=False,
            )
        return status, report

    def test_missing_tool_blocks_before_sentinel_child(self) -> None:
        with tempfile.TemporaryDirectory(prefix="dev-check-sentinel-") as temp:
            sentinel = Path(temp) / "started"
            check = {
                "label": "must not start",
                "argv": [sys.executable, "-c", "import pathlib,sys; pathlib.Path(sys.argv[1]).write_text('started')", str(sentinel)],
                "cwd": ".",
                "inputs": ["src/**/*.py"],
                "reuse": False,
                "tools": ["absent-command"],
                "timeout_seconds": 5,
            }
            tools = {
                "git": copy.deepcopy(dev_check.load_config(ROOT)["tools"]["git"]),
                "absent-command": {"version_argv": ["dev-check-tool-that-does-not-exist", "--version"], "version_regex": "^TOOL ([0-9.]+)"},
            }
            repo = self.make_sentinel_repo(check, tools=tools)
            try:
                status, report = self.run_and_capture(repo)
                self.assertEqual(status, 2)
                self.assertIn("missing tool: absent-command", report["preflight"]["errors"])
                self.assertFalse(sentinel.exists())
            finally:
                repo.close()

    def test_wrong_tool_version_blocks_before_sentinel_child(self) -> None:
        with tempfile.TemporaryDirectory(prefix="dev-check-fake-bin-") as temp:
            bin_dir = Path(temp) / "bin"
            bin_dir.mkdir()
            actual_git = shutil.which("git")
            self.assertIsNotNone(actual_git)
            (bin_dir / "git").symlink_to(actual_git)
            fake = bin_dir / "fake-tool"
            fake.write_text("#!/bin/sh\nprintf 'TOOL 1.0\\n'\n", encoding="utf-8")
            fake.chmod(0o700)
            sentinel = Path(temp) / "started"
            check = {
                "label": "must not start",
                "argv": [sys.executable, "-c", "import pathlib,sys; pathlib.Path(sys.argv[1]).write_text('started')", str(sentinel)],
                "cwd": ".",
                "inputs": ["src/**/*.py"],
                "reuse": False,
                "tools": ["fake-tool"],
                "timeout_seconds": 5,
            }
            tools = {
                "git": copy.deepcopy(dev_check.load_config(ROOT)["tools"]["git"]),
                "fake-tool": {"version_argv": ["fake-tool", "--version"], "version_regex": "^TOOL ([0-9.]+)", "exact": "2.0"},
            }
            repo = self.make_sentinel_repo(check, tools=tools)
            try:
                with mock.patch.dict(os.environ, {"PATH": str(bin_dir)}):
                    status, report = self.run_and_capture(repo)
                self.assertEqual(status, 2)
                self.assertTrue(any("wrong version for fake-tool" in item for item in report["preflight"]["errors"]))
                self.assertFalse(sentinel.exists())
            finally:
                repo.close()

    def test_cargo_formatter_executes_pinned_toolchain_under_override_and_receipts_it(self) -> None:
        with tempfile.TemporaryDirectory(prefix="dev-check-fake-cargo-") as temp:
            temp_root = Path(temp)
            bin_dir = temp_root / "bin"
            bin_dir.mkdir()
            actual_git = shutil.which("git")
            self.assertIsNotNone(actual_git)
            (bin_dir / "git").symlink_to(actual_git)
            capture = temp_root / "cargo-invocation.txt"
            fake_cargo = bin_dir / "cargo"
            fake_cargo.write_text(
                "#!/bin/sh\n"
                "if [ \"$1\" = \"+1.97.1\" ] && [ \"$2\" = \"--version\" ]; then printf 'cargo 1.97.1 (fixture)\\n'; exit 0; fi\n"
                "{ printf 'toolchain=%s\\n' \"$RUSTUP_TOOLCHAIN\"; printf 'auto=%s\\n' \"$RUSTUP_AUTO_INSTALL\"; printf 'formatter=%s\\n' \"${RUSTFMT-unset}\"; printf 'argv=%s\\n' \"$*\"; } > \"$DEV_CHECK_CARGO_CAPTURE\"\n"
                "if [ -n \"${RUSTFMT-}\" ]; then exit 46; fi\n"
                "if [ \"$1\" = \"+1.97.1\" ] && [ \"$2\" = \"fmt\" ]; then exit 0; fi\n"
                "printf 'wrong toolchain selection\\n' >&2; exit 45\n",
                encoding="utf-8",
            )
            fake_cargo.chmod(0o700)
            fake_rustfmt = bin_dir / "rustfmt"
            fake_rustfmt.write_text(
                "#!/bin/sh\nif [ \"$1\" = \"+1.97.1\" ] && [ \"$2\" = \"--version\" ]; then printf 'rustfmt 1.97.1\\n'; exit 0; fi\nexit 45\n",
                encoding="utf-8",
            )
            fake_rustfmt.chmod(0o700)

            check = copy.deepcopy(dev_check.load_config(ROOT)["checks"]["cargo-format"])
            check["pin_manifest"] = False
            check["inputs"] = ["src/**/*.py"]
            repo = TemporaryRepository(check, tools={
                "git": copy.deepcopy(dev_check.load_config(ROOT)["tools"]["git"]),
                "cargo": copy.deepcopy(dev_check.load_config(ROOT)["tools"]["cargo"]),
                "rustfmt": copy.deepcopy(dev_check.load_config(ROOT)["tools"]["rustfmt"]),
            })
            repo.config["checks"]["cargo-format"] = check
            repo.config["profiles"]["rust"]["fast"] = ["cargo-format"]
            repo.write_config()
            try:
                with mock.patch.dict(
                    os.environ,
                    {
                        "PATH": str(bin_dir),
                        "RUSTUP_TOOLCHAIN": "stable-override",
                        "RUSTFMT": "/unverified/formatter",
                        "DEV_CHECK_CARGO_CAPTURE": str(capture),
                    },
                ):
                    status, report = dev_check.run_profile(
                        repo.root,
                        repo.config,
                        "rust",
                        "fast",
                        environment="local",
                        reuse=False,
                    )
                self.assertEqual(status, 0)
                self.assertEqual(report["results"][0]["verdict"], "PASS")
                invocation = capture.read_text(encoding="utf-8")
                self.assertIn("toolchain=stable-override", invocation)
                self.assertIn("auto=0", invocation)
                self.assertIn("formatter=unset", invocation)
                self.assertIn("argv=+1.97.1 fmt --all -- --check", invocation)
                receipt = json.loads((repo.root / report["receipt_dir"] / "cargo-format.json").read_text(encoding="utf-8"))
                self.assertEqual(receipt["argv"][0:3], ["cargo", "+1.97.1", "fmt"])
                self.assertEqual(receipt["tool_facts"]["cargo"]["version"], "1.97.1")
                self.assertTrue(receipt["reuse_eligible"])
                self.assertIn("RUSTUP_TOOLCHAIN", receipt["environment_facts"])
                self.assertEqual(receipt["environment_facts"]["RUSTUP_TOOLCHAIN"]["value"], "stable-override")
                self.assertEqual(receipt["environment_facts"]["RUSTUP_AUTO_INSTALL"]["value"], "0")
                self.assertEqual(receipt["environment_facts"]["RUSTFMT"], {
                    "present": False, "sha256": None, "dispatcher_override": True,
                })
            finally:
                repo.close()

    def test_missing_environment_name_blocks_without_echoing_values_or_starting_child(self) -> None:
        with tempfile.TemporaryDirectory(prefix="dev-check-env-") as temp:
            sentinel = Path(temp) / "started"
            check = {
                "label": "must not start",
                "argv": [sys.executable, "-c", "import pathlib,sys; pathlib.Path(sys.argv[1]).write_text('started')", str(sentinel)],
                "cwd": ".",
                "inputs": ["src/**/*.py"],
                "reuse": False,
                "tools": [],
                "required_env": ["DEV_CHECK_REQUIRED_SENTINEL"],
                "timeout_seconds": 5,
            }
            repo = self.make_sentinel_repo(check)
            try:
                with mock.patch.dict(os.environ, {}, clear=False):
                    os.environ.pop("DEV_CHECK_REQUIRED_SENTINEL", None)
                    status, report = self.run_and_capture(repo)
                self.assertEqual(status, 2)
                self.assertTrue(any("DEV_CHECK_REQUIRED_SENTINEL" in item for item in report["preflight"]["errors"]))
                self.assertNotIn("SENSITIVE", json.dumps(report))
                self.assertFalse(sentinel.exists())
            finally:
                repo.close()

    def test_symlink_and_permission_failures_do_not_repair_or_start_child(self) -> None:
        with tempfile.TemporaryDirectory(prefix="dev-check-output-") as temp:
            base = Path(temp)
            outside = base / "outside"
            outside.mkdir(mode=0o700)
            os.chmod(outside, 0o700)
            repo = TemporaryRepository({
                "label": "must not start",
                "argv": [sys.executable, "-c", "raise SystemExit(99)"],
                "cwd": ".",
                "inputs": ["src/**/*.py"],
                "reuse": False,
                "tools": [],
            })
            try:
                receipt_parent = repo.root / "target" / "dev-check"
                receipt_parent.symlink_to(outside, target_is_directory=True)
                before = stat_mode(outside)
                status, report = self.run_and_capture(repo)
                self.assertEqual(status, 2)
                self.assertTrue(any("symlink" in item for item in report["preflight"]["errors"]))
                self.assertEqual(stat_mode(outside), before)
                receipt_parent.unlink()
                receipt_parent.mkdir(mode=0o755)
                os.chmod(receipt_parent, 0o755)
                before = stat_mode(receipt_parent)
                status, report = self.run_and_capture(repo)
                self.assertEqual(status, 2)
                self.assertTrue(any("private" in item for item in report["preflight"]["errors"]))
                self.assertEqual(stat_mode(receipt_parent), before)
            finally:
                repo.close()

    def test_docker_daemon_unavailable_blocks_before_sentinel_child(self) -> None:
        with tempfile.TemporaryDirectory(prefix="dev-check-docker-") as temp:
            bin_dir = Path(temp) / "bin"
            bin_dir.mkdir()
            actual_git = shutil.which("git")
            self.assertIsNotNone(actual_git)
            (bin_dir / "git").symlink_to(actual_git)
            fake_docker = bin_dir / "docker"
            fake_docker.write_text(
                "#!/bin/sh\nif [ \"$1\" = info ]; then echo unavailable >&2; exit 1; fi\nprintf 'Docker version 28.0.0, build test\\n'\n",
                encoding="utf-8",
            )
            fake_docker.chmod(0o700)
            sentinel = Path(temp) / "started"
            check = {
                "label": "must not start",
                "argv": [sys.executable, "-c", "import pathlib,sys; pathlib.Path(sys.argv[1]).write_text('started')", str(sentinel)],
                "cwd": ".",
                "inputs": ["src/**/*.py"],
                "reuse": False,
                "tools": ["docker"],
                "docker_daemon": True,
                "timeout_seconds": 5,
            }
            tools = {
                "git": copy.deepcopy(dev_check.load_config(ROOT)["tools"]["git"]),
                "docker": copy.deepcopy(dev_check.load_config(ROOT)["tools"]["docker"]),
            }
            repo = self.make_sentinel_repo(check, tools=tools)
            try:
                with mock.patch.dict(os.environ, {"PATH": str(bin_dir)}):
                    status, report = self.run_and_capture(repo)
                self.assertEqual(status, 2)
                self.assertTrue(any("Docker daemon unavailable" in item for item in report["preflight"]["errors"]))
                self.assertFalse(sentinel.exists())
            finally:
                repo.close()

    def test_eaddrinuse_is_distinct_and_foreign_listener_is_preserved(self) -> None:
        with tempfile.TemporaryDirectory(prefix="dev-check-port-") as temp:
            sentinel = Path(temp) / "started"
            check = {
                "label": "must not start",
                "argv": [sys.executable, "-c", "import pathlib,sys; pathlib.Path(sys.argv[1]).write_text('started')", str(sentinel)],
                "cwd": ".",
                "inputs": ["src/**/*.py"],
                "reuse": False,
                "tools": [],
                "ports": [{"name": "DEV_CHECK_TEST_PORT", "default": 1}],
                "timeout_seconds": 5,
            }
            repo = self.make_sentinel_repo(check)
            listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            try:
                listener.bind(("127.0.0.1", 0))
                listener.listen(1)
                port = listener.getsockname()[1]
                with mock.patch.dict(os.environ, {"DEV_CHECK_TEST_PORT": str(port)}):
                    status, report = self.run_and_capture(repo)
                self.assertEqual(status, 2)
                self.assertTrue(any(f"port {port} unavailable: observed errno EADDRINUSE" in item for item in report["preflight"]["errors"]))
                self.assertFalse(sentinel.exists())
                self.assertEqual(listener.getsockname()[1], port)
            finally:
                listener.close()
                repo.close()

    def test_eacces_and_eperm_are_reported_without_calling_them_busy(self) -> None:
        with tempfile.TemporaryDirectory(prefix="dev-check-denied-port-") as temp:
            sentinel = Path(temp) / "started"
            check = {
                "label": "must not start",
                "argv": [sys.executable, "-c", "import pathlib,sys; pathlib.Path(sys.argv[1]).write_text('started')", str(sentinel)],
                "cwd": ".",
                "inputs": ["src/**/*.py"],
                "reuse": False,
                "tools": [],
                "ports": [{"name": "DEV_CHECK_TEST_PORT", "default": 12345}],
                "timeout_seconds": 5,
            }
            repo = self.make_sentinel_repo(check)
            try:
                for number, name in ((errno.EACCES, "EACCES"), (errno.EPERM, "EPERM")):
                    with mock.patch.object(dev_check.socket, "socket", side_effect=OSError(number, "denied")):
                        status, report = self.run_and_capture(repo)
                    self.assertEqual(status, 2)
                    self.assertTrue(any(f"observed errno {name}" in item for item in report["preflight"]["errors"]))
                    self.assertFalse(any("unavailable: observed errno EADDRINUSE" in item for item in report["preflight"]["errors"]))
                    self.assertFalse(sentinel.exists())
            finally:
                repo.close()

    def test_explicit_api_web_port_collision_blocks_before_child_and_same_name_deduplicates(self) -> None:
        with tempfile.TemporaryDirectory(prefix="dev-check-port-collision-") as temp:
            sentinel = Path(temp) / "started"
            check = {
                "label": "must not start on colliding endpoints",
                "argv": [sys.executable, "-c", "import pathlib,sys; pathlib.Path(sys.argv[1]).write_text('started')", str(sentinel)],
                "cwd": ".",
                "inputs": ["src/**/*.py"],
                "reuse": False,
                "tools": [],
                "ports": [
                    {"name": "CANDIDATE_E2E_API_PORT", "default": 38182},
                    {"name": "CANDIDATE_E2E_WEB_PORT", "default": 33001},
                ],
                "timeout_seconds": 5,
            }
            repo = self.make_sentinel_repo(check)
            try:
                with mock.patch.dict(
                    os.environ,
                    {"CANDIDATE_E2E_API_PORT": "38182", "CANDIDATE_E2E_WEB_PORT": "38182"},
                ):
                    status, report = self.run_and_capture(repo)
                self.assertEqual(status, 2)
                self.assertIn(
                    "configured endpoint port collision: CANDIDATE_E2E_API_PORT and CANDIDATE_E2E_WEB_PORT both use 38182",
                    report["preflight"]["errors"],
                )
                self.assertFalse(sentinel.exists())
                duplicate = dev_check.normalize_ports([
                    {"name": "CANDIDATE_E2E_API_PORT", "default": 38182},
                    {"name": "CANDIDATE_E2E_API_PORT", "default": 38182},
                ])
                self.assertEqual(len(duplicate), 1)
                self.assertFalse(duplicate[0].get("collision"))
                qa_requirements = dev_check.requirement_context(
                    dev_check.load_config(ROOT),
                    "qa",
                    "fast",
                    dev_check.selected_ids(dev_check.load_config(ROOT), "qa", "fast"),
                )
                self.assertFalse(qa_requirements["docker_daemon"])
                self.assertFalse(qa_requirements["ports"])
            finally:
                repo.close()


def stat_mode(path: Path) -> tuple[int, int]:
    info = path.lstat()
    return info.st_uid, info.st_mode & 0o777


class ReceiptTests(unittest.TestCase):
    def make_repo(self, check: dict | None = None) -> TemporaryRepository:
        return TemporaryRepository(check)

    def run_profile(self, repo: TemporaryRepository, *, reuse: bool = False) -> tuple[int, dict]:
        with contextlib.redirect_stdout(io.StringIO()):
            return dev_check.run_profile(
                repo.root,
                repo.config,
                "qa",
                "fast",
                environment="local",
                reuse=reuse,
            )

    def test_symlink_and_ancestor_symlink_inputs_fail_closed_without_reading_targets_or_reusing(self) -> None:
        repo = self.make_repo()
        try:
            outside = repo.root / "target" / "external.py"
            secret_bytes = b"OUTSIDE_SENSITIVE_BYTES_SENTINEL\n"
            outside.write_bytes(secret_bytes)
            source = repo.root / "src" / "input.py"
            source.unlink()
            source.symlink_to(outside)
            real_open = os.open

            def guarded_open(path, flags, *args, **kwargs):
                candidate = Path(path)
                if not candidate.is_absolute() and kwargs.get("dir_fd") is not None:
                    descriptor_path = Path(f"/proc/self/fd/{kwargs['dir_fd']}")
                    if descriptor_path.exists():
                        candidate = Path(os.readlink(descriptor_path)) / candidate
                try:
                    if candidate.resolve(strict=False) == outside.resolve(strict=False):
                        raise AssertionError("dispatcher attempted to open symlink target contents")
                except OSError:
                    pass
                return real_open(path, flags, *args, **kwargs)

            with mock.patch.object(dev_check.os, "open", side_effect=guarded_open):
                first_status, first = self.run_profile(repo)
                self.assertEqual(first_status, 1)
                self.assertEqual(first["results"][0]["verdict"], "FAILED_INPUT_COVERAGE")
                self.assertIsNone(first["results"][0]["original_exit_code"])
                outside.write_text("this is invalid Python (\n", encoding="utf-8")
                second_status, second = self.run_profile(repo, reuse=True)
            self.assertEqual(second_status, 1)
            self.assertNotEqual(second["results"][0]["verdict"], "REUSED")
            self.assertEqual(second["results"][0]["verdict"], "FAILED_INPUT_COVERAGE")
            for report in (first, second):
                run_dir = repo.root / report["receipt_dir"]
                serialized = "\n".join(path.read_text(encoding="utf-8") for path in run_dir.iterdir() if path.is_file())
                self.assertNotIn("OUTSIDE_SENSITIVE_BYTES_SENTINEL", serialized)
                receipt = json.loads((run_dir / "python-syntax.json").read_text(encoding="utf-8"))
                self.assertFalse(receipt["reuse_eligible"])
                self.assertIsNone(receipt["original_receipt"])
        finally:
            repo.close()

        with tempfile.TemporaryDirectory(prefix="dev-check-ancestor-symlink-") as temp:
            outside_dir = Path(temp) / "external-src"
            outside_dir.mkdir()
            (outside_dir / "input.py").write_text("ANCESTOR_TARGET_SECRET_SENTINEL\n", encoding="utf-8")
            ancestor_repo = self.make_repo()
            try:
                (ancestor_repo.root / "src").rename(ancestor_repo.root / "src-original")
                (ancestor_repo.root / "src").symlink_to(outside_dir, target_is_directory=True)
                status, report = self.run_profile(ancestor_repo)
                self.assertEqual(status, 1)
                self.assertEqual(report["results"][0]["verdict"], "FAILED_INPUT_COVERAGE")
                serialized = "\n".join(
                    path.read_text(encoding="utf-8")
                    for path in (ancestor_repo.root / report["receipt_dir"]).iterdir()
                    if path.is_file()
                )
                self.assertNotIn("ANCESTOR_TARGET_SECRET_SENTINEL", serialized)
            finally:
                ancestor_repo.close()

        coverage_repo = self.make_repo()
        try:
            fifo = coverage_repo.root / "src" / "pipe.py"
            os.mkfifo(fifo)
            records, safe = dev_check.file_fingerprint(coverage_repo.root, ["src/pipe.py"])
            self.assertFalse(safe)
            self.assertEqual(records[0]["state"], "non-regular")
            with mock.patch.object(dev_check.os, "open", side_effect=PermissionError(errno.EACCES, "denied")):
                records, safe = dev_check.file_fingerprint(coverage_repo.root, ["src/input.py"])
            self.assertFalse(safe)
            self.assertEqual(records[0]["state"], "unreadable:EACCES")
        finally:
            coverage_repo.close()

    def test_failure_receipt_has_bounded_private_redacted_diagnostic_and_reviewable_facts(self) -> None:
        secret = "qa-password-sentinel-value-93"
        check = {
            "label": "diagnostic failure fixture",
            "argv": [
                sys.executable,
                "-B",
                "-c",
                "import os,sys; print('ACTUAL_FAILURE_REASON', file=sys.stderr); print(os.environ['QA_PASSWORD']); print('data/phase0/raw/provider-payload.json'); print('x' * 24000); raise SystemExit(7)",
            ],
            "cwd": ".",
            "inputs": ["src/**/*.py"],
            "reuse": True,
            "reuse_kind": "python-syntax",
            "tools": ["python3"],
            "fingerprint_env": ["QA_PASSWORD"],
            "timeout_seconds": 5,
        }
        repo = self.make_repo(check)
        output = io.StringIO()
        try:
            with mock.patch.dict(os.environ, {"QA_PASSWORD": secret}):
                with contextlib.redirect_stdout(output):
                    status, report = dev_check.run_profile(
                        repo.root, repo.config, "qa", "fast", environment="local", reuse=False
                    )
            self.assertEqual(status, 1)
            result = report["results"][0]
            self.assertEqual(result["original_exit_code"], 7)
            metadata = result["diagnostic"]
            diagnostic_path = repo.root / metadata["path"]
            diagnostic = diagnostic_path.read_bytes()
            self.assertEqual(stat_mode(diagnostic_path)[1], 0o600)
            self.assertLessEqual(len(diagnostic), dev_check.DIAGNOSTIC_MAX_BYTES)
            self.assertTrue(metadata["truncated"])
            self.assertEqual(metadata["sha256"], hashlib.sha256(diagnostic).hexdigest())
            self.assertIn(b"ACTUAL_FAILURE_REASON", diagnostic)
            self.assertIn(b"[provider-data-path]", diagnostic)
            self.assertNotIn(b"data/phase0/raw/provider-payload.json", diagnostic)
            self.assertNotIn(secret.encode(), diagnostic)
            self.assertIn(metadata["path"], output.getvalue())
            receipt = json.loads((repo.root / report["receipt_dir"] / "python-syntax.json").read_text(encoding="utf-8"))
            self.assertEqual(receipt["original_exit_code"], 7)
            self.assertEqual(receipt["diagnostic"]["sha256"], metadata["sha256"])
            self.assertEqual(receipt["tool_facts"]["python3"]["version"], dev_check.tool_version("python3", dev_check.load_config(ROOT)["tools"]["python3"])["version"])
            self.assertEqual(receipt["environment_facts"]["QA_PASSWORD"], {"present": True, "value": "<redacted>"})
            self.assertNotIn(secret, json.dumps(receipt))
        finally:
            repo.close()

    def test_same_inputs_reuse_original_receipt_without_running_child_again(self) -> None:
        with tempfile.TemporaryDirectory(prefix="dev-check-reuse-") as temp:
            count_path = Path(temp) / "target" / "call-count.txt"
            count_path.parent.mkdir(parents=True)
            check = {
                "label": "counted deterministic syntax",
                "argv": [
                    sys.executable,
                    "-B",
                    "-c",
                    "import ast,pathlib,sys; ast.parse(pathlib.Path(sys.argv[1]).read_text()); p=pathlib.Path(sys.argv[2]); p.write_text(str(int(p.read_text())+1) if p.exists() else '1')",
                    "src/input.py",
                    str(count_path),
                ],
                "cwd": ".",
                "inputs": ["src/**/*.py", "Cargo.lock", "settings.json"],
                "reuse": True,
                "reuse_kind": "python-syntax",
                "tools": ["python3"],
                "timeout_seconds": 20,
            }
            repo = self.make_repo(check)
            try:
                first_status, first = self.run_profile(repo)
                self.assertEqual(first_status, 0)
                self.assertEqual(count_path.read_text(encoding="utf-8"), "1")
                first_receipt = first["receipt_dir"] + "/python-syntax.json"
                first_data = json.loads((repo.root / first_receipt).read_text(encoding="utf-8"))
                self.assertEqual(first_data["verdict"], "PASS")
                self.assertTrue(first_data["reuse_eligible"])

                second_status, second = self.run_profile(repo, reuse=True)
                self.assertEqual(second_status, 0)
                self.assertEqual(second["results"][0]["verdict"], "REUSED")
                self.assertEqual(count_path.read_text(encoding="utf-8"), "1")
                reused_receipt = json.loads(
                    (repo.root / second["receipt_dir"] / "python-syntax.json").read_text(encoding="utf-8")
                )
                self.assertEqual(reused_receipt["verdict"], "REUSED")
                self.assertEqual(reused_receipt["original_receipt"], first_receipt)
                self.assertEqual(reused_receipt["original_target_head"], first_data["target_head"])
                self.assertFalse(reused_receipt["reuse_eligible"])
                self.assertEqual(stat_mode(repo.root / "target" / "dev-check" / first["run_id"])[1], 0o700)
                self.assertEqual(stat_mode(repo.root / first_receipt)[1], 0o600)
            finally:
                repo.close()

    def test_each_declared_input_tool_env_argv_cwd_and_config_change_invalidates(self) -> None:
        repo = self.make_repo()
        try:
            check = repo.config["checks"]["python-syntax"]
            changes = dev_check.collect_changes(repo.root, "HEAD")
            tools = {"python3": {"path": sys.executable, "version": "3.12.13", "ok": True}}
            argv = [sys.executable, "-B", "-c", "pass"]
            cwd = repo.root
            with mock.patch.dict(os.environ, {"DEV_CHECK_TEST_ENV": "one"}):
                original, _, _ = dev_check.fingerprint(repo.root, "python-syntax", check, argv, cwd, changes, tools)
                (repo.root / "docs" / "note.md").write_text("unrelated docs change\n", encoding="utf-8")
                unchanged_docs, _, _ = dev_check.fingerprint(repo.root, "python-syntax", check, argv, cwd, changes, tools)
                self.assertEqual(original, unchanged_docs)

                (repo.root / "src" / "input.py").write_text("value = 2\n", encoding="utf-8")
                source_change, _, _ = dev_check.fingerprint(repo.root, "python-syntax", check, argv, cwd, changes, tools)
                self.assertNotEqual(original, source_change)
                (repo.root / "src" / "input.py").write_text("value = 1\n", encoding="utf-8")

                (repo.root / "Cargo.lock").write_text("lock-version = 2\n", encoding="utf-8")
                lock_change, _, _ = dev_check.fingerprint(repo.root, "python-syntax", check, argv, cwd, changes, tools)
                self.assertNotEqual(original, lock_change)
                (repo.root / "Cargo.lock").write_text("lock-version = 1\n", encoding="utf-8")

                config_file = repo.root / "scripts" / "qa" / "check-profiles.json"
                old_config = config_file.read_text(encoding="utf-8")
                config_file.write_text(old_config + " ", encoding="utf-8")
                config_change, _, _ = dev_check.fingerprint(repo.root, "python-syntax", check, argv, cwd, changes, tools)
                self.assertNotEqual(original, config_change)
                config_file.write_text(old_config, encoding="utf-8")

                tool_change, _, _ = dev_check.fingerprint(
                    repo.root,
                    "python-syntax",
                    check,
                    argv,
                    cwd,
                    changes,
                    {"python3": {"path": sys.executable, "version": "3.12.14", "ok": True}},
                )
                self.assertNotEqual(original, tool_change)

                with mock.patch.dict(os.environ, {"DEV_CHECK_TEST_ENV": "two"}):
                    env_change, _, _ = dev_check.fingerprint(repo.root, "python-syntax", check, argv, cwd, changes, tools)
                self.assertNotEqual(original, env_change)

                argv_change, _, _ = dev_check.fingerprint(repo.root, "python-syntax", check, argv + ["new-arg"], cwd, changes, tools)
                self.assertNotEqual(original, argv_change)
                cwd_change, _, _ = dev_check.fingerprint(repo.root, "python-syntax", check, argv, repo.root / "src", changes, tools)
                self.assertNotEqual(original, cwd_change)
                ci_change, _, _ = dev_check.fingerprint(repo.root, "python-syntax", check, argv, cwd, changes, tools, "ci")
                self.assertNotEqual(original, ci_change)
        finally:
            repo.close()

    def test_unrelated_docs_do_not_invalidate_rust_formatter_fingerprint(self) -> None:
        repo = self.make_repo()
        try:
            rust_file = repo.root / "src" / "lib.rs"
            rust_file.write_text("pub fn stable() {}\n", encoding="utf-8")
            changes = dev_check.collect_changes(repo.root, "HEAD")
            check = copy.deepcopy(dev_check.load_config(ROOT)["checks"]["cargo-format"])
            argv = [str(part) for part in check["argv"]]
            tools = {
                "cargo": {"path": "/fake/cargo", "version": "1.97.1", "ok": True},
                "rustfmt": {"path": "/fake/rustfmt", "version": "1.97.1", "ok": True},
            }
            original, _, safe = dev_check.fingerprint(repo.root, "cargo-format", check, argv, repo.root, changes, tools)
            self.assertTrue(safe)
            (repo.root / "docs" / "note.md").write_text("unrelated documentation edit\n", encoding="utf-8")
            docs_only, _, docs_safe = dev_check.fingerprint(repo.root, "cargo-format", check, argv, repo.root, changes, tools)
            self.assertTrue(docs_safe)
            self.assertEqual(original, docs_only)
        finally:
            repo.close()

    def test_deleted_only_syntax_input_is_an_explicit_skip_not_a_pass(self) -> None:
        with tempfile.TemporaryDirectory(prefix="dev-check-deleted-syntax-") as temp:
            sentinel = Path(temp) / "child-started"
            check = {
                "label": "changed Python AST syntax",
                "argv": [sys.executable, "-B", "-c", "import pathlib,sys; pathlib.Path(sys.argv[1]).write_text('started')", str(sentinel), "{changed_python}"],
                "cwd": ".",
                "changed_patterns": ["src/**/*.py"],
                "inputs": ["src/**/*.py"],
                "reuse": True,
                "reuse_kind": "python-syntax",
                "tools": ["python3"],
                "timeout_seconds": 5,
            }
            repo = self.make_repo(check)
            repo.config["profiles"]["qa"]["fast"] = ["python-syntax"]
            repo.write_config()
            (repo.root / "src" / "input.py").unlink()
            try:
                tombstones, coverage_safe = dev_check.file_fingerprint(repo.root, ["src/input.py"])
                self.assertTrue(coverage_safe)
                self.assertEqual(tombstones, [{"path": "src/input.py", "state": "missing"}])
                status, report = self.run_profile(repo)
                self.assertEqual(status, 0)
                self.assertEqual(report["status"], "PASS_WITH_SKIPS")
                self.assertEqual(report["results"][0]["verdict"], "SKIPPED_NO_APPLICABLE_INPUT")
                self.assertFalse(sentinel.exists())
                receipt = json.loads((repo.root / report["receipt_dir"] / "python-syntax.json").read_text(encoding="utf-8"))
                self.assertEqual(receipt["verdict"], "SKIPPED_NO_APPLICABLE_INPUT")
                self.assertFalse(receipt["reuse_eligible"])
            finally:
                repo.close()

    def test_failed_partial_corrupt_runtime_timeout_and_source_mutation_receipts_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="dev-check-receipt-reject-") as temp:
            repo_root = Path(temp)
            (repo_root / "scripts" / "qa").mkdir(parents=True)
            (repo_root / "src").mkdir()
            shutil.copyfile(SCRIPT, repo_root / "scripts" / "qa" / "dev-check.py")
            (repo_root / "src" / "input.py").write_text("value = 1\n", encoding="utf-8")
            (repo_root / "target").mkdir(mode=0o700)
            os.chmod(repo_root / "target", 0o700)
            command(["git", "init", "-q"], repo_root)
            command(["git", "config", "user.email", "dev-check@example.invalid"], repo_root)
            command(["git", "config", "user.name", "dev-check-test"], repo_root)
            command(["git", "add", "."], repo_root)
            command(["git", "commit", "-qm", "baseline"], repo_root)
            base_config = {
                "schema_version": 1,
                "dispatcher_version": "test",
                "common_patterns": [],
                "path_rules": [],
                "tools": {},
                "profiles": {"qa": {"description": "test", "fast": ["python-syntax"], "focused": [], "integration": []}},
                "checks": {},
                "integration_requirements": {},
                "integration_plans": {},
            }
            (repo_root / "scripts" / "qa" / "check-profiles.json").write_text(
                json.dumps(base_config), encoding="utf-8"
            )
            changes = dev_check.collect_changes(repo_root, "HEAD")
            check = {
                "label": "timeout",
                "argv": [sys.executable, "-B", "-c", "import time; time.sleep(2)"],
                "cwd": ".",
                "inputs": ["src/**/*.py"],
                "reuse": True,
                "reuse_kind": "python-syntax",
                "tools": [],
                "timeout_seconds": 0.02,
            }
            timed = dev_check.execute_check(
                repo_root, "python-syntax", check, check["argv"], repo_root, changes, {}, reuse=False
            )
            self.assertEqual(timed["verdict"], "FAILED_TIMEOUT")
            self.assertTrue(timed["timed_out"])
            run_id, run_dir = dev_check.make_run_dir(repo_root)
            dev_check.persist_run(repo_root, run_id, run_dir, "qa", "fast", "local", command(["git", "rev-parse", "HEAD"], repo_root), [timed], ["python-syntax"], False)
            self.assertIsNone(dev_check.find_reusable_receipt(repo_root, "python-syntax", timed["input_fingerprint"]))

            mutation_check = {
                **check,
                "label": "mutates source",
                "argv": [sys.executable, "-B", "-c", "import pathlib; pathlib.Path('src/input.py').write_text('value = 2\\n')"],
                "timeout_seconds": 5,
            }
            mutation = dev_check.execute_check(
                repo_root, "python-syntax", mutation_check, mutation_check["argv"], repo_root, changes, {}, reuse=False
            )
            self.assertEqual(mutation["original_exit_code"], 0)
            self.assertEqual(mutation["verdict"], "FAILED_INPUT_CHANGED")
            self.assertFalse(mutation["inputs_stable"])

            for verdict, eligible, runtime, partial in (
                ("FAILED", False, False, False),
                ("PASS", True, True, False),
                ("PASS", True, False, True),
            ):
                payload = {
                    "schema_version": 1,
                    "check_id": "python-syntax",
                    "input_fingerprint": "a" * 64,
                    "verdict": verdict,
                    "original_exit_code": 0 if verdict == "PASS" else 1,
                    "reuse_eligible": eligible,
                    "timed_out": False,
                    "inputs_stable": True,
                    "runtime_dependent": runtime,
                    "partial": partial,
                    "target_head": "b" * 40,
                    "run_id": "test-run",
                    "argv": ["python3"],
                    "cwd": str(repo_root),
                    "started_at": "2026-10-13T00:00:00Z",
                    "completed_at": "2026-10-13T00:00:01Z",
                    "duration_ms": 1000,
                    "evidence_paths": [],
                    "reuse_kind": "python-syntax",
                }
                manifest = {"schema_version": 1, "complete": True, "status": "PASS", "run_id": "test-run", "target_head": "b" * 40, "results": [{"check_id": "python-syntax", "verdict": "PASS"}]}
                self.assertFalse(dev_check.receipt_valid(payload, check_id="python-syntax", expected_fingerprint="a" * 64, run_manifest=manifest))
            corrupt = {"not": "a receipt"}
            self.assertFalse(dev_check.receipt_valid(corrupt, check_id="python-syntax", expected_fingerprint="a" * 64, run_manifest={}))
            malformed_id, malformed_dir = dev_check.make_run_dir(repo_root)
            dev_check.write_private_json(
                malformed_dir / "run.json",
                {
                    "schema_version": 1,
                    "run_id": malformed_id,
                    "complete": True,
                    "status": "PASS",
                    "target_head": command(["git", "rev-parse", "HEAD"], repo_root),
                    "results": [{"check_id": "python-syntax", "verdict": "PASS"}],
                },
            )
            malformed_path = malformed_dir / "python-syntax.json"
            malformed_path.write_text("{not-json", encoding="utf-8")
            os.chmod(malformed_path, 0o600)
            self.assertIsNone(dev_check.find_reusable_receipt(repo_root, "python-syntax", "a" * 64))

    def test_failed_first_check_stops_later_child_and_keeps_original_exit(self) -> None:
        with tempfile.TemporaryDirectory(prefix="dev-check-first-failure-") as temp:
            later = Path(temp) / "later-started"
            first = {
                "label": "first failure",
                "argv": [sys.executable, "-B", "-c", "raise SystemExit(7)"],
                "cwd": ".",
                "inputs": ["src/**/*.py"],
                "reuse": False,
                "tools": [],
                "timeout_seconds": 5,
            }
            repo = self.make_repo(first)
            repo.config["profiles"]["qa"]["fast"].append("second")
            repo.config["checks"]["second"] = {
                "label": "must not start",
                "argv": [sys.executable, "-B", "-c", "import pathlib,sys; pathlib.Path(sys.argv[1]).write_text('started')", str(later)],
                "cwd": ".",
                "inputs": ["src/**/*.py"],
                "reuse": False,
                "tools": [],
                "timeout_seconds": 5,
            }
            repo.write_config()
            try:
                status, report = self.run_profile(repo)
                self.assertEqual(status, 1)
                self.assertEqual(len(report["results"]), 1)
                self.assertEqual(report["results"][0]["original_exit_code"], 7)
                self.assertEqual(report["first_failure"]["verdict"], "FAILED")
                self.assertFalse(later.exists())
                manifest = json.loads((repo.root / report["receipt_dir"] / "run.json").read_text(encoding="utf-8"))
                self.assertEqual(manifest["status"], "PARTIAL")
            finally:
                repo.close()


if __name__ == "__main__":
    unittest.main()
