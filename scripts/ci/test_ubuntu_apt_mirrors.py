from __future__ import annotations

import importlib.util
import os
import socket
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


def _load_sibling(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


mirrors = _load_sibling("apt_mirror_subject", "configure_ubuntu_apt_mirrors.py")
load_workflow = _load_sibling("apt_mirror_workflow_contract", "test_ci_contract.py").load_workflow


DEFAULT_TEXT = (
    "# runner mirrors\n"
    f"{mirrors.AZURE}\tpriority:1\n"
    "\n"
    f"{mirrors.ARCHIVE}\tpriority:2\n"
    f"{mirrors.SECURITY}\tpriority:3\n"
)
EXPECTED_TEXT = (
    "# runner mirrors\n"
    f"{mirrors.AZURE}\tpriority:3\n"
    "\n"
    f"{mirrors.ARCHIVE}\tpriority:1\n"
    f"{mirrors.SECURITY}\tpriority:2\n"
)


class AptMirrorTransformTests(unittest.TestCase):
    def test_exact_default_transform_preserves_order_comments_and_line_endings(self) -> None:
        transformed, status = mirrors.transform_mirror_config(DEFAULT_TEXT)
        self.assertEqual(status, "UPDATED")
        self.assertEqual(transformed, EXPECTED_TEXT)
        self.assertEqual(
            [line.split("\t", 1)[0] for line in transformed.splitlines() if line.startswith("http")],
            [mirrors.AZURE, mirrors.ARCHIVE, mirrors.SECURITY],
        )

        crlf = DEFAULT_TEXT.replace("\n", "\r\n").rstrip("\r\n")
        transformed_crlf, status = mirrors.transform_mirror_config(crlf)
        self.assertEqual(status, "UPDATED")
        self.assertEqual(transformed_crlf, EXPECTED_TEXT.replace("\n", "\r\n").rstrip("\r\n"))

    def test_transform_is_idempotent_and_preserves_comment_only_bytes(self) -> None:
        transformed, _ = mirrors.transform_mirror_config(DEFAULT_TEXT)
        again, status = mirrors.transform_mirror_config(transformed)
        self.assertEqual((again, status), (transformed, "UNCHANGED"))
        comments = "# " + mirrors.AZURE + "\n\n"
        self.assertEqual(mirrors.transform_mirror_config(comments), (comments, "UNCHANGED"))

    def test_missing_azure_and_lookalike_host_are_unchanged(self) -> None:
        without_azure = f"{mirrors.ARCHIVE}\tpriority:9\n"
        lookalike = "http://azure.archive.ubuntu.com.evil/ubuntu/\tpriority:1\n"
        embedded = f"https://mirror.example/path/{mirrors.AZURE}\tpriority:1\n"
        for text in (without_azure, lookalike, embedded):
            with self.subTest(text=text):
                self.assertEqual(mirrors.transform_mirror_config(text), (text, "UNCHANGED"))

    def test_azure_config_refuses_missing_duplicate_malformed_and_unknown_records(self) -> None:
        bad_configs = {
            "missing canonical": f"{mirrors.AZURE}\tpriority:1\n",
            "duplicate": DEFAULT_TEXT + f"{mirrors.AZURE}\tpriority:5\n",
            "malformed priority": DEFAULT_TEXT.replace("priority:1", "priority:+1"),
            "malformed azure record": DEFAULT_TEXT.replace(
                f"{mirrors.AZURE}\tpriority:1", f"{mirrors.AZURE} priority:1"
            ),
            "leading whitespace azure": DEFAULT_TEXT.replace(mirrors.AZURE, " " + mirrors.AZURE),
            "unknown mirror": DEFAULT_TEXT + "https://mirror.example/ubuntu/\tpriority:4\n",
            "unknown metadata": DEFAULT_TEXT.replace("priority:2", "priority:2\tregion:west"),
        }
        for name, text in bad_configs.items():
            with self.subTest(name=name), self.assertRaises(mirrors.MirrorConfigError):
                mirrors.transform_mirror_config(text)

    def test_oversize_and_invalid_utf8_refuse(self) -> None:
        with self.assertRaises(mirrors.MirrorConfigError):
            mirrors._transform_bytes(b"x" * (mirrors.MAX_CONFIG_BYTES + 1))
        with self.assertRaises(mirrors.MirrorConfigError):
            mirrors._transform_bytes(b"\xff")
        with self.assertRaises(mirrors.MirrorConfigError):
            mirrors.transform_mirror_config("\ud800")

    def test_large_numeric_priority_is_bounded_without_integer_conversion(self) -> None:
        text = (
            f"{mirrors.AZURE}\tpriority:{'9' * 5000}\n"
            f"{mirrors.ARCHIVE}\tpriority:2\n"
            f"{mirrors.SECURITY}\tpriority:3\n"
        )
        transformed, status = mirrors.transform_mirror_config(text)
        self.assertEqual(status, "UPDATED")
        self.assertEqual(
            transformed,
            f"{mirrors.AZURE}\tpriority:3\n"
            f"{mirrors.ARCHIVE}\tpriority:1\n"
            f"{mirrors.SECURITY}\tpriority:2\n",
        )


class AptMirrorFileTests(unittest.TestCase):
    def setUp(self) -> None:
        # These unit cases must never spawn a process, make a network connection,
        # or change ownership. They exercise only the helper and private temp files.
        self._effects = [
            mock.patch.object(subprocess, "Popen", side_effect=AssertionError("process blocked")),
            mock.patch.object(subprocess, "run", side_effect=AssertionError("process blocked")),
            mock.patch.object(socket, "socket", side_effect=AssertionError("socket blocked")),
            mock.patch.object(socket, "create_connection", side_effect=AssertionError("socket blocked")),
            mock.patch.object(os, "chown", side_effect=AssertionError("ownership change blocked")),
            mock.patch.object(os, "fchown", side_effect=AssertionError("ownership change blocked")),
        ]
        for effect in self._effects:
            effect.start()
            self.addCleanup(effect.stop)

    def _write_config(self, path: Path, text: str = DEFAULT_TEXT, mode: int = 0o640) -> None:
        path.write_text(text, encoding="utf-8")
        path.chmod(mode)

    def test_atomic_file_update_preserves_mode_fsyncs_and_closes_descriptors(self) -> None:
        with tempfile.TemporaryDirectory(prefix="apt mirrors ") as directory:
            path = Path(directory) / "apt-mirrors.txt"
            self._write_config(path)
            opened: list[int] = []
            fsync_calls: list[int] = []
            original_open = os.open
            original_fsync = os.fsync

            def recording_open(*args, **kwargs):
                fd = original_open(*args, **kwargs)
                opened.append(fd)
                return fd

            def recording_fsync(fd: int) -> None:
                fsync_calls.append(fd)
                original_fsync(fd)

            with mock.patch.object(mirrors.os, "open", side_effect=recording_open), mock.patch.object(
                mirrors.os, "fsync", side_effect=recording_fsync
            ):
                self.assertEqual(mirrors.configure_mirror_file(path), "UPDATED")

            self.assertEqual(path.read_text(encoding="utf-8"), EXPECTED_TEXT)
            self.assertEqual(path.stat().st_mode & 0o777, 0o640)
            self.assertGreaterEqual(len(fsync_calls), 2)
            self.assertTrue(opened)
            for fd in set(opened):
                with self.assertRaises(OSError):
                    os.fstat(fd)
            self.assertEqual(list(Path(directory).glob(".apt-mirrors-*.tmp")), [])

    def test_missing_file_is_a_noop_and_unsafe_file_or_symlink_refuses(self) -> None:
        with tempfile.TemporaryDirectory(prefix="apt mirrors ") as directory:
            root = Path(directory)
            self.assertEqual(mirrors.configure_mirror_file(root / "missing"), "NOOP_MISSING")

            unsafe = root / "writable"
            self._write_config(unsafe, mode=0o666)
            with self.assertRaises(mirrors.MirrorConfigError):
                mirrors.configure_mirror_file(unsafe)

            nonregular = root / "directory"
            nonregular.mkdir(mode=0o700)
            with self.assertRaises(mirrors.MirrorConfigError):
                mirrors.configure_mirror_file(nonregular)

            unsafe_parent = root / "unsafe-parent"
            unsafe_parent.mkdir(mode=0o700)
            unsafe_parent.chmod(0o777)
            unsafe_parent_file = unsafe_parent / "apt-mirrors.txt"
            self._write_config(unsafe_parent_file)
            with self.assertRaises(mirrors.MirrorConfigError):
                mirrors.configure_mirror_file(unsafe_parent_file)

            target = root / "target"
            self._write_config(target)
            link = root / "link"
            link.symlink_to(target)
            with self.assertRaises((mirrors.MirrorConfigError, OSError)):
                mirrors.configure_mirror_file(link)

    def test_symlinked_parent_refuses(self) -> None:
        with tempfile.TemporaryDirectory(prefix="apt mirrors ") as directory:
            root = Path(directory)
            real_parent = root / "real"
            real_parent.mkdir(mode=0o700)
            self._write_config(real_parent / "apt-mirrors.txt")
            link_parent = root / "link"
            link_parent.symlink_to(real_parent, target_is_directory=True)
            with self.assertRaises((mirrors.MirrorConfigError, OSError)):
                mirrors.configure_mirror_file(link_parent / "apt-mirrors.txt")

    def test_source_replacement_before_publication_refuses_and_cleans_owned_temp(self) -> None:
        with tempfile.TemporaryDirectory(prefix="apt mirrors ") as directory:
            path = Path(directory) / "apt-mirrors.txt"
            self._write_config(path)
            original_recheck = mirrors._recheck_target
            replaced = False

            def replace_source_then_check(*args, **kwargs):
                nonlocal replaced
                if not replaced:
                    path.unlink()
                    self._write_config(path, DEFAULT_TEXT)
                    replaced = True
                return original_recheck(*args, **kwargs)

            with mock.patch.object(mirrors, "_recheck_target", side_effect=replace_source_then_check):
                with self.assertRaises(mirrors.MirrorConfigError):
                    mirrors.configure_mirror_file(path)
            self.assertTrue(replaced)
            self.assertEqual(path.read_text(encoding="utf-8"), DEFAULT_TEXT)
            self.assertEqual(list(Path(directory).glob(".apt-mirrors-*.tmp")), [])

    def test_parent_replacement_before_publication_refuses_and_cleans_owned_temp(self) -> None:
        with tempfile.TemporaryDirectory(prefix="apt mirrors ") as directory:
            root = Path(directory)
            parent = root / "parent"
            parent.mkdir(mode=0o700)
            path = parent / "apt-mirrors.txt"
            self._write_config(path)
            moved = root / "moved"
            original_recheck = mirrors._recheck_target
            replaced = False

            def replace_parent_then_check(*args, **kwargs):
                nonlocal replaced
                if not replaced:
                    parent.rename(moved)
                    parent.mkdir(mode=0o700)
                    self._write_config(parent / path.name, DEFAULT_TEXT)
                    replaced = True
                return original_recheck(*args, **kwargs)

            with mock.patch.object(mirrors, "_recheck_target", side_effect=replace_parent_then_check):
                with self.assertRaises(mirrors.MirrorConfigError):
                    mirrors.configure_mirror_file(path)
            self.assertTrue(replaced)
            self.assertEqual((parent / path.name).read_text(encoding="utf-8"), DEFAULT_TEXT)
            self.assertEqual(list(moved.glob(".apt-mirrors-*.tmp")), [])


class AptMirrorEntrypointAndWorkflowTests(unittest.TestCase):
    def test_main_checks_fixed_target_and_environment_before_file_access(self) -> None:
        env = {"GITHUB_ACTIONS": "true", "RUNNER_OS": "Linux"}
        with mock.patch.object(mirrors.sys, "argv", ["configure_ubuntu_apt_mirrors.py"]), \
             mock.patch.object(mirrors.sys, "platform", "linux"), \
             mock.patch.object(mirrors.platform, "machine", return_value="x86_64"), \
             mock.patch.object(mirrors.os, "geteuid", return_value=0), \
             mock.patch.dict(os.environ, env, clear=True), \
             mock.patch.object(mirrors, "configure_mirror_file", return_value="NOOP_MISSING") as configure, \
             mock.patch("builtins.print") as print_output:
            self.assertEqual(mirrors.main(), 0)
        configure.assert_called_once_with(Path("/etc/apt/apt-mirrors.txt"))
        print_output.assert_called_once_with("NOOP_MISSING")

        with mock.patch.object(mirrors, "configure_mirror_file", side_effect=AssertionError("host access")), \
             mock.patch.dict(os.environ, {}, clear=True), \
             mock.patch.object(mirrors.sys, "argv", ["configure_ubuntu_apt_mirrors.py"]), \
             mock.patch("builtins.print"):
            self.assertEqual(mirrors.main(), 2)

    def test_workflow_registers_helper_before_both_real_installs_and_keeps_gates(self) -> None:
        _, workflow = load_workflow("ci.yml")
        helper_call = (
            'sudo --preserve-env=GITHUB_ACTIONS,RUNNER_OS /usr/bin/python3 '
            '"$GITHUB_WORKSPACE/scripts/ci/configure_ubuntu_apt_mirrors.py"'
        )
        web_steps = workflow["jobs"]["web"]["steps"]
        playwright = next(step for step in web_steps if step.get("id") == "playwright-install")
        self.assertEqual(playwright["run"].count(helper_call), 1)
        self.assertLess(playwright["run"].index("started=$SECONDS"), playwright["run"].index(helper_call))
        self.assertLess(playwright["run"].index(helper_call), playwright["run"].index("npx playwright install --with-deps chromium"))

        pg = workflow["jobs"]["postgres-integration-validation"]
        self.assertEqual(pg["runs-on"], "ubuntu-24.04")
        self.assertEqual(pg["timeout-minutes"], "90")
        self.assertEqual(len(pg["steps"]), 16)
        libfaketime = next(step for step in pg["steps"] if step.get("id") == "libfaketime")
        self.assertEqual(libfaketime["run"].count(helper_call), 1)
        self.assertLess(libfaketime["run"].index(helper_call), libfaketime["run"].index("sudo apt-get update"))
        self.assertIn("libfaketime=0.9.10-2.1", libfaketime["run"])
        self.assertIn("sudo apt-get install --yes --no-install-recommends", libfaketime["run"])

        policy = next(step for step in workflow["jobs"]["policy"]["steps"] if "test_ubuntu_apt_mirrors" in step.get("run", ""))
        policy_lines = policy["run"].splitlines()
        self.assertIn(
            "python -m unittest scripts.ci.test_ci_contract scripts.ci.test_cache_evidence "
            "scripts.ci.test_ubuntu_apt_mirrors -v",
            policy_lines,
        )
        required = workflow["jobs"]["required"]
        self.assertEqual(required.get("if"), "always()")
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
                "paper-image-integration",
                "r4-verification-build",
            },
        )


if __name__ == "__main__":
    unittest.main()
