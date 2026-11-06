from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import signal
import stat
import sys
import threading
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("r5-paper-native-os.py")
SPEC = importlib.util.spec_from_file_location("r5_paper_native_os", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
runner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runner
SPEC.loader.exec_module(runner)


class ArtifactAndInventoryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.target = self.root / "cargo-target"
        (self.target / "debug/deps").mkdir(parents=True)
        self.elf = self.target / "debug/deps/job_queue-deadbeef"
        self.elf.write_bytes(b"\x7fELF" + b"fixture")
        self.elf.chmod(0o755)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def artifact_line(self, path: Path | None = None) -> str:
        return json.dumps({
            "reason": "compiler-artifact",
            "target": {"name": "job_queue", "kind": ["lib"], "src_path": str(runner.LIB_SOURCE)},
            "profile": {"test": True},
            "executable": str(path or self.elf),
        })

    def test_compiler_artifact_is_exact_job_queue_test_elf(self) -> None:
        path = runner.parse_compiler_artifact([self.artifact_line()], self.target)
        self.assertEqual(path, self.elf)
        digest = runner.sha256_file(path)
        fd, identity = runner.open_pinned_elf(path, self.target, digest)
        os.close(fd)
        self.assertEqual(identity["links"], 1)

    def test_duplicate_and_foreign_artifacts_fail_closed(self) -> None:
        with self.assertRaisesRegex(runner.RunnerError, "AMBIGUOUS"):
            runner.parse_compiler_artifact([self.artifact_line(), self.artifact_line()], self.target)
        foreign = self.root / "foreign"
        foreign.write_bytes(b"\x7fELFoutside")
        with self.assertRaisesRegex(runner.RunnerError, "OUTSIDE_ALLOWED_ROOT"):
            runner.parse_compiler_artifact([self.artifact_line(foreign)], self.target)

    def test_symlink_substitution_and_hash_or_inode_drift_fail(self) -> None:
        link = self.target / "debug/deps/substituted"
        link.symlink_to(self.elf)
        with self.assertRaisesRegex(runner.RunnerError, "SYMLINK"):
            runner.parse_compiler_artifact([self.artifact_line(link)], self.target)
        digest = runner.sha256_file(self.elf)
        fd, identity = runner.open_pinned_elf(self.elf, self.target, digest)
        os.close(fd)
        self.elf.write_bytes(b"\x7fELFchanged")
        with self.assertRaisesRegex(runner.RunnerError, "DRIFT|HASH_MISMATCH"):
            runner.open_pinned_elf(self.elf, self.target, digest, identity)

    def test_all_six_exact_ignored_tests_are_required_once(self) -> None:
        self.assertEqual(len(runner.TESTS), 6)
        output = "\n".join(f"{name}: test" for name in runner.TESTS)
        self.assertEqual(set(runner.parse_inventory(output)), set(runner.TESTS))
        with self.assertRaisesRegex(runner.RunnerError, "MISSING_OR_DUPLICATE"):
            runner.parse_inventory(f"{runner.TESTS[0]}: test")
        with self.assertRaisesRegex(runner.RunnerError, "MISSING_OR_DUPLICATE"):
            runner.parse_inventory(output + f"\n{runner.TESTS[0]}: test")

    def test_cleanup_namespace_allows_only_owned_native_fixture_classes(self) -> None:
        nonce = "12345678-1234-4234-8234-123456789abc"
        for prefix in ("native-readback-", "native-action-", "native-chain-state-", "native-chain-result-"):
            self.assertTrue(runner.fixture_child_name(prefix + nonce))
        for name in ("native-action-../x", "native-readback-" + nonce.upper(),
                     "other-" + nonce, "native-action-" + nonce + "/child",
                     "native-action-12345678-1234-1234-8234-123456789abc"):
            self.assertFalse(runner.fixture_child_name(name))

    def test_success_requires_named_pass_and_one_pass_summary(self) -> None:
        name = runner.TESTS[0]
        good = f"test {name} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored;"
        runner.validate_named_result(name, good, 0)
        for output, code in (
            ("test result: ok. 0 passed; 0 failed; 0 ignored;", 0),
            (f"test {name} ... FAILED\ntest result: FAILED. 0 passed; 1 failed; 0 ignored;", 0),
            (good, 1),
            (good.replace("1 passed", "2 passed"), 0),
        ):
            with self.subTest(output=output, code=code), self.assertRaises(runner.RunnerError):
                runner.validate_named_result(name, output, code)

    def test_hosted_gate_rejects_local_invocation_without_running_commands(self) -> None:
        with self.assertRaisesRegex(runner.RunnerError, "GITHUB_ACTIONS_REQUIRED"):
            runner.hosted_paths({"GITHUB_ACTIONS": "false"})


class OwnedFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.parent = Path(self.temp.name) / "r5-paper-checker-integration"
        self.fixture = runner.OwnedFixture(self.parent, os.getuid(), os.getgid())

    def tearDown(self) -> None:
        if self.parent.exists():
            # Test cleanup only: a leftover means the ownership guard was the
            # subject of the assertion, so remove this uniquely created temp.
            import shutil
            shutil.rmtree(self.parent)
        self.temp.cleanup()

    def test_new_private_parent_gets_nonce_and_only_its_native_tree_is_removed(self) -> None:
        self.fixture.create()
        st = os.stat(self.parent, follow_symlinks=False)
        self.assertEqual(stat.S_IMODE(st.st_mode), 0o700)
        self.assertEqual(os.stat(self.parent / ".owner-nonce").st_mode & 0o777, 0o400)
        child = self.parent / "native-readback-11111111-1111-4111-8111-111111111111"
        child.mkdir(mode=0o700)
        (child / "fixture.bin").write_bytes(b"owned test bytes")
        runner._remove_owned_tree(self.fixture.fd, child.name, os.getuid())
        self.fixture.cleanup()
        self.assertFalse(self.parent.exists())

    def test_preexisting_parent_is_rejected_without_mutation(self) -> None:
        self.parent.mkdir(mode=0o700)
        marker = self.parent / "keep"
        marker.write_bytes(b"preexisting")
        before = marker.read_bytes()
        with self.assertRaisesRegex(runner.RunnerError, "PREEXISTED"):
            self.fixture.create()
        self.assertEqual(marker.read_bytes(), before)
        self.assertEqual(list(self.parent.iterdir()), [marker])

    def test_unknown_entry_blocks_owned_parent_removal(self) -> None:
        self.fixture.create()
        marker = self.parent / "unowned"
        marker.write_bytes(b"preserve")
        with self.assertRaisesRegex(runner.RunnerError, "UNOWNED_ENTRY"):
            self.fixture.cleanup()
        self.assertEqual(marker.read_bytes(), b"preserve")

    def test_replaced_parent_is_never_removed_as_owned(self) -> None:
        self.fixture.create()
        displaced = self.parent.with_name("r5-paper-checker-integration-original")
        os.rename(self.parent, displaced)
        self.parent.mkdir(mode=0o700)
        marker = self.parent / "replacement"
        marker.write_bytes(b"keep replacement")
        with self.assertRaisesRegex(runner.RunnerError, "IDENTITY_DRIFT"):
            self.fixture.cleanup()
        self.assertEqual(marker.read_bytes(), b"keep replacement")
        marker.unlink()
        self.parent.rmdir()
        os.rename(displaced, self.parent)
        self.fixture.cleanup()

    def test_dangling_symlink_preexisting_parent_is_preserved(self) -> None:
        self.parent.symlink_to(self.parent.with_name("missing-target"))
        with self.assertRaisesRegex(runner.RunnerError, "PREEXISTED"):
            self.fixture.create()
        self.assertTrue(self.parent.is_symlink())


class ChildLifecycleTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def test_output_is_capped_and_reported_as_typed_failure(self) -> None:
        script = "import sys; sys.stdout.write('x'*4096); sys.stdout.flush()"
        with self.assertRaisesRegex(runner.RunnerError, "OUTPUT_LIMIT"):
            runner.run_logged([sys.executable, "-c", script], cwd=self.root, env={"PATH": os.environ["PATH"]},
                              stdout_path=self.root / "out", stderr_path=self.root / "err", timeout=5,
                              max_bytes=128)
        self.assertLessEqual((self.root / "out").stat().st_size, 128)
        self.assertEqual(stat.S_IMODE((self.root / "out").stat().st_mode), 0o600)

    def test_timeout_terminates_and_reaps_only_owned_child(self) -> None:
        pid_file = self.root / "pid"
        script = "import os,time,pathlib; pathlib.Path(os.environ['PID_FILE']).write_text(str(os.getpid())); time.sleep(30)"
        with self.assertRaisesRegex(runner.RunnerError, "TIMEOUT"):
            runner.run_logged([sys.executable, "-c", script], cwd=self.root,
                              env={"PATH": os.environ["PATH"], "PID_FILE": str(pid_file)},
                              stdout_path=self.root / "out", stderr_path=self.root / "err", timeout=0.2)
        pid = int(pid_file.read_text())
        with self.assertRaises(ProcessLookupError):
            os.kill(pid, 0)

    def test_interrupt_terminates_and_reaps_owned_session(self) -> None:
        pid_file = self.root / "interrupt-pid"
        script = "import os,time,pathlib; pathlib.Path(os.environ['PID_FILE']).write_text(str(os.getpid())); time.sleep(30)"
        timer = threading.Timer(0.15, lambda: os.kill(os.getpid(), signal.SIGINT))
        timer.start()
        try:
            with self.assertRaisesRegex(runner.RunnerError, "INTERRUPTED"):
                runner.run_logged([sys.executable, "-c", script], cwd=self.root,
                                  env={"PATH": os.environ["PATH"], "PID_FILE": str(pid_file)},
                                  stdout_path=self.root / "interrupt.out", stderr_path=self.root / "interrupt.err",
                                  timeout=5)
        finally:
            timer.cancel()
        pid = int(pid_file.read_text())
        with self.assertRaises(ProcessLookupError):
            os.kill(pid, 0)


class ResultProtocolTests(unittest.TestCase):
    def test_root_result_must_match_exact_source_elf_and_test_set(self) -> None:
        value = {"schema": "r5-paper-native-os-result-v1", "source_sha": "a" * 40,
                 "elf_sha256": "b" * 64, "tests": [{"name": n, "status": "PASS"} for n in runner.TESTS],
                 "cleanup": "PASS", "status": "PASS", "error": None}
        self.assertEqual(runner._root_result(json.dumps(value), "a" * 40, "b" * 64), value)
        value["tests"].pop()
        with self.assertRaisesRegex(runner.RunnerError, "TESTS_INVALID"):
            runner._root_result(json.dumps(value), "a" * 40, "b" * 64)
        with self.assertRaisesRegex(runner.RunnerError, "MALFORMED"):
            runner._root_result("not json", "a" * 40, "b" * 64)

    def test_handoff_head_reader_checks_loose_and_packed_git_refs(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            repo = Path(temp) / "repo"
            gitdir = Path(temp) / "gitdir"
            repo.mkdir()
            gitdir.mkdir()
            (repo / ".git").write_text(f"gitdir: {gitdir}\n", encoding="utf-8")
            (gitdir / "HEAD").write_text("ref: refs/heads/r5-test\n", encoding="ascii")
            ref = gitdir / "refs/heads/r5-test"
            ref.parent.mkdir(parents=True)
            ref.write_text("a" * 40 + "\n", encoding="ascii")
            self.assertEqual(runner._git_head_without_process(repo), "a" * 40)
            ref.unlink()
            (gitdir / "packed-refs").write_text("# pack-refs with: peeled fully-peeled\n" + "b" * 40 + " refs/heads/r5-test\n", encoding="ascii")
            self.assertEqual(runner._git_head_without_process(repo), "b" * 40)
            (gitdir / "packed-refs").write_text("c" * 40 + " refs/heads/other\n", encoding="ascii")
            with self.assertRaisesRegex(runner.RunnerError, "GIT_IDENTITY_FAILED"):
                runner._git_head_without_process(repo)

    def test_evidence_directory_is_private_and_anchored_under_runner_temp(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            parent = Path(temp)
            parent.chmod(0o700)
            evidence = parent / runner.EVIDENCE_LEAF
            evidence.mkdir(mode=0o700)
            runner.verify_evidence(evidence, parent, os.getuid())
            evidence.chmod(0o750)
            with self.assertRaisesRegex(runner.RunnerError, "EVIDENCE_DIRECTORY_INVALID"):
                runner.verify_evidence(evidence, parent, os.getuid())
            evidence.chmod(0o700)
            with tempfile.TemporaryDirectory() as unrelated:
                with self.assertRaisesRegex(runner.RunnerError, "OUTSIDE_RUNNER_TEMP"):
                    runner.verify_evidence(evidence, Path(unrelated), os.getuid())


if __name__ == "__main__":
    unittest.main()
