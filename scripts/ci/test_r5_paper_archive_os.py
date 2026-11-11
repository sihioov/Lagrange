from __future__ import annotations

import contextlib
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import types
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location("r5_paper_archive_os", Path(__file__).with_name("r5-paper-archive-os.py"))
archive = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = archive
SPEC.loader.exec_module(archive)
runner = archive.native


def good_output(name):
    return f"test {name} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 100 filtered out; finished in 0.01s\n"


class RegistrationTests(unittest.TestCase):
    def test_existing_workflow_runs_native_then_archive_in_one_required_stage(self):
        text = (archive.ROOT / ".github/workflows/ci.yml").read_text()
        marker = "      - name: Verify Root-only native Paper OS fixtures\n        run: |\n"
        self.assertEqual(text.count(marker), 1)
        block = text.split(marker, 1)[1].split("      - ", 1)[0]
        commands = [line.strip() for line in block.splitlines() if line.strip()]
        self.assertEqual(commands, ["set -euo pipefail", "python3 scripts/ci/r5-paper-native-os.py",
                                    "python3 scripts/ci/r5-paper-archive-os.py"])
        self.assertIn("scripts.ci.test_r5_paper_native_os scripts.ci.test_r5_paper_archive_os -v", text)

    def test_fixed_market_data_profile_and_original_limits(self):
        self.assertEqual((runner.PACKAGE_NAME, runner.TARGET_NAME), ("market-data", "market_data"))
        self.assertEqual(runner.LIB_SOURCE, archive.ROOT / "crates/market-data/src/lib.rs")
        self.assertEqual(len(archive.TESTS), 16)
        self.assertEqual(len(set(archive.TESTS)), 16)
        self.assertTrue(all(x.startswith("paper_archive_sources::closure::tests::actual_root_") for x in archive.TESTS))
        self.assertEqual(sum("retained_parquet" in x for x in archive.TESTS), 4)
        self.assertEqual((runner.BUILD_TIMEOUT, runner.TEST_TIMEOUT, runner.ROOT_HELPER_TIMEOUT), (1800, 120, 360))
        self.assertEqual(runner.EVIDENCE_LEAF, "r5-paper-archive-os")
        self.assertEqual(hashlib.sha256(archive.HELPER.read_bytes()).hexdigest(), archive.HELPER_SHA256)

    def test_every_inventory_name_required_exactly_once(self):
        output = "\n".join(f"{name}: test" for name in archive.TESTS)
        self.assertEqual(len(runner.parse_inventory(output)), 16)
        for name in archive.TESTS:
            for changed in (output.replace(f"{name}: test", ""), output + f"\n{name}: test"):
                with self.assertRaises(runner.RunnerError):
                    runner.parse_inventory(changed)

    def test_all_sixteen_strict_named_results_and_foreign_or_malformed_rejected(self):
        for name in archive.TESTS:
            good = good_output(name)
            runner.validate_named_result(name, good, 0)
            for changed, status in ((good, 1), (good + f"test foreign ... ok\n", 0),
                                    (good + good, 0), (good.replace("0 ignored", "1 ignored"), 0),
                                    (good + "test result: FAILED. 0 passed; 1 failed; 0 ignored;\n", 0),
                                    (good.replace(name, "foreign"), 0),
                                    (good + "test malformed header without verdict\n", 0)):
                with self.assertRaises(runner.RunnerError):
                    runner.validate_named_result(name, changed, status)

    def test_exact_target_artifact_and_foreign_duplicate_or_wrong_source_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            elf = target / "market_data-fixture"
            elf.write_bytes(b"\x7fELFfixture")
            elf.chmod(0o755)
            row = {"reason": "compiler-artifact", "target": {"name": "market_data", "kind": ["lib"],
                    "src_path": str(runner.LIB_SOURCE)}, "profile": {"test": True}, "executable": str(elf)}
            self.assertEqual(runner.parse_compiler_artifact([json.dumps(row)], target), elf)
            with self.assertRaises(runner.RunnerError):
                runner.parse_compiler_artifact([json.dumps(row)] * 2, target)
            for key, value in (("name", "job_queue"), ("kind", ["bin"]), ("src_path", "foreign")):
                bad = copy.deepcopy(row)
                bad["target"][key] = value
                with self.assertRaises(runner.RunnerError):
                    runner.parse_compiler_artifact([json.dumps(bad)], target)

    def test_success_json_requires_all_sixteen_ordered_and_no_duplicate_keys(self):
        value = {"schema": runner.RESULT_SCHEMA, "source_sha": "a" * 40, "elf_sha256": "b" * 64,
                 "status": "PASS", "error": None, "cleanup": "PASS",
                 "tests": [{"name": n, "status": "PASS"} for n in archive.TESTS]}
        runner._root_result(json.dumps(value), "a" * 40, "b" * 64)
        for bad in (dict(value, tests=value["tests"][:-1]), dict(value, tests=value["tests"][::-1]),
                    dict(value, cleanup="NOT_CONFIRMED"), dict(value, source_sha="c" * 40)):
            with self.assertRaises(runner.RunnerError):
                runner._root_result(json.dumps(bad), "a" * 40, "b" * 64)
        duplicate = json.dumps(value)[:-1] + ',"status":"PASS"}'
        with self.assertRaises(runner.RunnerError):
            runner._root_result(duplicate, "a" * 40, "b" * 64)


class SourcePinTests(unittest.TestCase):
    def test_original_fd_read_rejects_replacement_links_modes_and_oversize(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "source.py"
            path.write_bytes(b"fixed-source")
            path.chmod(0o600)
            data, identity, digest = archive._read_source(path)
            self.assertEqual(data, b"fixed-source")
            self.assertEqual(digest, hashlib.sha256(data).hexdigest())
            with self.assertRaises(ValueError):
                archive._read_source(path, "0" * 64)
            link = path.with_name("link")
            link.symlink_to(path)
            with self.assertRaises(OSError):
                archive._read_source(link)
            link.unlink()
            os.link(path, link)
            with self.assertRaises(ValueError):
                archive._read_source(path)
            link.unlink()
            path.chmod(0o666)
            with self.assertRaises(ValueError):
                archive._read_source(path)
            path.chmod(0o600)
            real_read = os.read
            replaced = False
            def replace(fd, count):
                nonlocal replaced
                block = real_read(fd, count)
                if not replaced:
                    replacement = path.with_name("replacement")
                    replacement.write_bytes(data)
                    replacement.chmod(0o600)
                    os.replace(replacement, path)
                    replaced = True
                return block
            with mock.patch.object(archive.os, "read", side_effect=replace):
                with self.assertRaisesRegex(ValueError, "IDENTITY_DRIFT"):
                    archive._read_source(path)
            path.write_bytes(b"x" * (1024 * 1024 + 1))
            with self.assertRaises(ValueError):
                archive._read_source(path)

    def pins(self):
        identity = dict(archive._HELPER_IDENTITY)
        return [{"path": p.relative_to(archive.ROOT).as_posix(), "sha256": "a" * 64,
                 "identity": dict(identity)} for p in (archive.SCRIPT, archive.HELPER, archive.ARCHIVE_SOURCE)]

    def test_handoff_requires_entire_fixed_typed_source_packet(self):
        pins = self.pins()
        elf = {"device": 1, "inode": 2, "size": 3, "mode": 0o755, "uid": 1000, "gid": 1000, "links": 1}
        packet = {"elf": elf, "elf_metadata": {"mtime_ns": 1, "ctime_ns": 2}, "sources": pins}
        with mock.patch.object(archive, "_source_pins", return_value=pins), \
                mock.patch.object(archive, "_PINNED_SOURCES", None):
            self.assertEqual(archive.decode_identity(packet), elf)
            for changed in (dict(packet, sources=pins[::-1]), dict(packet, sources=[]),
                            dict(packet, extra="private"), dict(packet, elf_metadata={"mtime_ns": True, "ctime_ns": 2})):
                with self.assertRaises(runner.RunnerError):
                    archive.decode_identity(changed)
            for value in (True, "1", None, -1):
                bad = copy.deepcopy(packet)
                bad["sources"][0]["identity"]["links"] = value
                with self.assertRaises(runner.RunnerError):
                    archive.decode_identity(bad)

    def test_held_elf_metadata_rejects_same_byte_timestamp_change(self):
        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.object(archive, "_PINNED_ELF_METADATA", None):
            target = Path(directory)
            path = target / "market_data-fixture"
            path.write_bytes(b"\x7fELFfixture")
            path.chmod(0o755)
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            fd, identity = archive.open_archive_elf(path, target, digest)
            os.close(fd)
            before = path.stat()
            os.utime(path, ns=(before.st_atime_ns, before.st_mtime_ns + 1_000_000))
            with self.assertRaisesRegex(runner.RunnerError, "IDENTITY_DRIFT"):
                archive.open_archive_elf(path, target, digest, identity)

    def test_source_metadata_or_identity_drift_stops_without_public_paths(self):
        first = self.pins()
        second = copy.deepcopy(first)
        second[0]["identity"]["inode"] += 1
        with mock.patch.object(archive, "_source_pins", side_effect=[first, second]), \
                mock.patch.object(archive, "_PINNED_SOURCES", None):
            archive.verify_archive_sources()
            with self.assertRaisesRegex(runner.RunnerError, "^ARCHIVE_SOURCE_IDENTITY_DRIFT$"):
                archive.verify_archive_sources()


class NamespaceTests(unittest.TestCase):
    def context(self, *, names=(), changed=False):
        stack = contextlib.ExitStack()
        def st(inode):
            return types.SimpleNamespace(st_dev=1, st_ino=inode, st_mode=stat.S_IFDIR | 0o755,
                                         st_uid=0, st_gid=0, st_nlink=3)
        values = {11: st(1), 12: st(2), 13: st(3)}
        named = {p: values[fd] for p, fd in zip(archive.ANCESTRY, values)}
        if changed:
            named[archive.ANCESTRY[-1]] = st(99)
        stack.enter_context(mock.patch.object(runner, "_no_symlink_path", side_effect=lambda path, **kw: path))
        stack.enter_context(mock.patch.object(archive.os, "open", side_effect=[11, 12, 13]))
        stack.enter_context(mock.patch.object(archive.os, "fstat", side_effect=lambda fd: values[fd]))
        stack.enter_context(mock.patch.object(archive.os, "stat", side_effect=lambda path, **kw: named[path]))
        stack.enter_context(mock.patch.object(runner, "_no_acl", return_value=True))
        stack.enter_context(mock.patch.object(archive.os, "listdir", return_value=list(names)))
        close = stack.enter_context(mock.patch.object(archive.os, "close"))
        unlink = stack.enter_context(mock.patch.object(archive.os, "unlink"))
        rmdir = stack.enter_context(mock.patch.object(archive.os, "rmdir"))
        return stack, close, unlink, rmdir

    def test_empty_namespace_rechecked_and_all_held_ancestry_closed(self):
        stack, close, unlink, rmdir = self.context()
        with stack:
            fixture = archive.ArchiveNamespace()
            fixture.create()
            fixture.before_test()
            fixture.after_test()
            fixture.cleanup()
            self.assertIsNone(fixture.fd)
            self.assertEqual([c.args[0] for c in close.call_args_list], [13, 12, 11])
            unlink.assert_not_called()
            rmdir.assert_not_called()

    def test_preexisting_or_residual_prefix_is_never_deleted(self):
        for suffix in ("foreign", "12345678-1234-4234-8234-123456789abc"):
            stack, close, unlink, rmdir = self.context(names=[archive.PREFIX + suffix])
            with stack:
                fixture = archive.ArchiveNamespace()
                with self.assertRaisesRegex(runner.RunnerError, "ARCHIVE_NAMESPACE_NOT_EMPTY"):
                    fixture.create()
                self.assertIsNone(fixture.fd)
                self.assertEqual(close.call_count, 3)
                unlink.assert_not_called()
                rmdir.assert_not_called()

    def test_changed_parent_identity_fails_and_closes_without_deletion(self):
        stack, close, unlink, rmdir = self.context(changed=True)
        with stack:
            fixture = archive.ArchiveNamespace()
            with self.assertRaisesRegex(runner.RunnerError, "IDENTITY_DRIFT"):
                fixture.create()
            self.assertEqual(close.call_count, 3)
            unlink.assert_not_called()
            rmdir.assert_not_called()

    def test_cleanup_residue_is_failure_even_when_test_drop_claims_success(self):
        stack, close, unlink, rmdir = self.context()
        with stack:
            fixture = archive.ArchiveNamespace()
            fixture.create()
            with mock.patch.object(archive.os, "listdir", return_value=[archive.PREFIX + "leftover"]):
                with self.assertRaises(runner.RunnerError):
                    fixture.after_test()
                with self.assertRaises(runner.RunnerError):
                    fixture.cleanup()
            self.assertIsNone(fixture.fd)
            unlink.assert_not_called()
            rmdir.assert_not_called()

    def test_close_failure_still_attempts_all_descriptors_and_cannot_pass(self):
        stack, close, _, _ = self.context()
        with stack:
            fixture = archive.ArchiveNamespace()
            fixture.create()
            close.side_effect = [OSError(5, "mock close"), None, None]
            with self.assertRaisesRegex(runner.RunnerError, "ARCHIVE_ANCESTRY_INVALID"):
                fixture.cleanup()
            self.assertEqual(close.call_count, 3)
            self.assertIsNone(fixture.fd)

    def test_partial_ancestry_open_failure_closes_the_unadopted_descriptor(self):
        stack, close, _, _ = self.context()
        with stack:
            stat_value = types.SimpleNamespace(st_dev=1, st_ino=1, st_mode=stat.S_IFDIR | 0o755,
                                               st_uid=0, st_gid=0, st_nlink=3)
            with mock.patch.object(archive.os, "fstat", side_effect=[stat_value, OSError(5, "mock stat")]):
                fixture = archive.ArchiveNamespace()
                with self.assertRaises(OSError):
                    fixture.create()
            self.assertEqual([c.args[0] for c in close.call_args_list], [12, 11])
            self.assertIsNone(fixture.fd)

    def test_acl_or_writable_ancestry_is_rejected_without_deletion(self):
        stack, close, unlink, rmdir = self.context()
        with stack, mock.patch.object(runner, "_no_acl", return_value=False):
            fixture = archive.ArchiveNamespace()
            with self.assertRaisesRegex(runner.RunnerError, "ARCHIVE_ANCESTRY_INVALID"):
                fixture.create()
            self.assertEqual(close.call_count, 3)
            unlink.assert_not_called()
            rmdir.assert_not_called()


class HostAndChildMockTests(unittest.TestCase):
    def test_real_log_owner_reaps_fake_child_after_start_callback_raises(self):
        with tempfile.TemporaryDirectory() as temp:
            base = Path(temp)
            process = mock.Mock(pid=12345)
            process.stdout.closed = False
            process.stderr.closed = False
            process.poll.return_value = None
            def waited(*args, **kwargs):
                process.poll.return_value = 0
                return 0
            process.wait.side_effect = waited
            with mock.patch.object(runner.subprocess, "Popen", return_value=process), \
                    mock.patch.object(runner.os, "killpg") as kill:
                with self.assertRaisesRegex(ValueError, "mock callback"):
                    runner.run_logged(["NEVER_EXECUTED"], cwd=base, env={},
                        stdout_path=base / "out", stderr_path=base / "err", timeout=1,
                        on_started=mock.Mock(side_effect=ValueError("mock callback")))
                self.assertTrue(process.wait.called)
                kill.assert_called_once_with(12345, runner.signal.SIGTERM)
                self.assertEqual((base / "out").stat().st_mode & 0o777, 0o600)

    def test_host_compile_inventory_and_cleared_root_route_are_mocked_and_serial(self):
        with tempfile.TemporaryDirectory() as temp:
            base = Path(temp)
            base.chmod(0o700)
            target = base / "target"
            target.mkdir()
            elf = target / "market_data-fixture"
            elf.write_bytes(b"\x7fELFfixture")
            elf.chmod(0o755)
            digest = hashlib.sha256(elf.read_bytes()).hexdigest()
            artifact = {"reason": "compiler-artifact", "target": {
                "name": "market_data", "kind": ["lib"], "src_path": str(runner.LIB_SOURCE)},
                "profile": {"test": True}, "executable": str(elf)}
            root = {"schema": runner.RESULT_SCHEMA, "source_sha": "a" * 40,
                    "elf_sha256": digest, "status": "PASS", "cleanup": "PASS", "error": None,
                    "tests": [{"name": n, "status": "PASS"} for n in archive.TESTS]}
            calls = []
            def run(argv, **kwargs):
                calls.append((argv, kwargs))
                if len(calls) == 1:
                    return 0, json.dumps(artifact), ""
                if len(calls) == 2:
                    return 0, "\n".join(f"{n}: test" for n in archive.TESTS), ""
                return 0, json.dumps(root), ""
            with mock.patch.object(archive, "_PINNED_ELF_METADATA", None), \
                    mock.patch.object(runner, "hosted_paths", return_value=(base, target, "a" * 40)), \
                    mock.patch.object(runner, "verify_candidate"), \
                    mock.patch.object(runner, "ROOT_IDENTITY_ENCODER", side_effect=lambda value: {"elf": value}), \
                    mock.patch.object(runner, "run_logged", side_effect=run), \
                    mock.patch.object(runner.Path, "is_file", return_value=True), \
                    mock.patch.object(runner.os, "access", return_value=True), \
                    mock.patch.object(runner.subprocess, "Popen") as popen:
                result = runner.run_hosted({})
                popen.assert_not_called()
            self.assertEqual(result["status"], "PASS")
            self.assertEqual(len(calls), 3)
            self.assertEqual(calls[0][0], ["cargo", "test", "--locked", "--package", "market-data",
                                         "--lib", "--no-run", "--message-format=json"])
            self.assertEqual(calls[0][1]["env"]["CARGO_BUILD_JOBS"], "2")
            self.assertEqual(calls[0][1]["env"]["CARGO_TARGET_DIR"], str(target))
            self.assertEqual(calls[2][0][:5], ["/usr/bin/sudo", "-n", "--", "/usr/bin/env", "-i"])
            self.assertEqual(set(calls[2][1]["env"]), {"PATH", "LANG", "LC_ALL", "TZ"})
            self.assertEqual(calls[2][1]["timeout"], 360)


class RootDriverMockTests(unittest.TestCase):
    def run_driver(self, failure=None):
        identity = {"device": 1, "inode": 2, "size": 3, "mode": 0o755, "uid": 1000, "gid": 1000, "links": 1}
        processes = []
        calls = []
        fixture = mock.Mock()
        self.last_fixture = fixture
        fixture.fd = 99
        fixture.cleanup.side_effect = lambda: setattr(fixture, "fd", None)
        def after():
            self.assertTrue(all(p.wait.called for p in processes))
        fixture.after_test.side_effect = after
        def run(argv, **kwargs):
            calls.append(argv)
            process = mock.Mock()
            process.poll.return_value = 0
            processes.append(process)
            if failure == "unreaped":
                process.poll.return_value = None
                process.wait.side_effect = RuntimeError("mock unreaped")
            kwargs["on_started"](process)
            if failure == "panic":
                process.poll.return_value = None
                raise ValueError("PRIVATE_SENTINEL_MUST_NOT_ESCAPE")
            process.wait()
            if failure == "timeout":
                raise runner.RunnerError("OWNED_COMMAND_TIMEOUT", 124)
            if failure == "cancel":
                raise runner.RunnerError("INTERRUPTED", 130)
            if failure == "nonzero":
                return 101, good_output(argv[-1]), ""
            return 0, good_output(argv[-1]), ""
        def terminate(process, **kw):
            process.poll.return_value = 0
            process.wait()
        directory = types.SimpleNamespace(st_mode=stat.S_IFDIR | 0o700, st_uid=1000)
        output = io.StringIO()
        self.last_output = output
        with mock.patch.dict(runner.os.environ, {"LANG": "C", "LC_ALL": "C", "TZ": "UTC", "SUDO_UID": "1000"}, clear=True), \
                mock.patch.object(runner.os, "geteuid", return_value=0), \
                mock.patch.object(runner.signal, "signal"), \
                mock.patch.object(runner, "ROOT_IDENTITY_DECODER", return_value=identity), \
                mock.patch.object(runner, "verify_candidate_after_handoff"), \
                mock.patch.object(runner, "_no_symlink_path", side_effect=lambda p, **kw: p), \
                mock.patch.object(runner.os, "stat", return_value=directory), \
                mock.patch.object(runner, "open_pinned_elf", return_value=(123, identity)), \
                mock.patch.object(runner.os, "close"), mock.patch.object(runner.os, "chown"), \
                mock.patch.object(runner, "FIXTURE_FACTORY", return_value=fixture), \
                mock.patch.object(runner, "run_logged", side_effect=run), \
                mock.patch.object(runner, "_terminate_owned", side_effect=terminate), \
                mock.patch.object(runner.subprocess, "Popen") as popen, \
                contextlib.redirect_stdout(output):
            args = ["12345678-1234-4234-8234-123456789abc", str(archive.ROOT), "a" * 40,
                    "/tmp/runner/target", "/tmp/runner/target/elf", "b" * 64,
                    json.dumps(identity), "/tmp/runner/r5-paper-archive-os", "c" * 64]
            status = runner._root_driver(args)
            popen.assert_not_called()
        self.assertNotIn("PRIVATE_SENTINEL", output.getvalue())
        self.assertTrue(all(p.wait.called for p in processes))
        fixture.cleanup.assert_called_once_with()
        return status, json.loads(output.getvalue()), calls, fixture

    def test_exact_sixteen_root_commands_serial_and_cleanup_after_reap(self):
        status, result, calls, fixture = self.run_driver()
        self.assertEqual(status, 0)
        self.assertEqual([c[-1] for c in calls], list(archive.TESTS))
        self.assertTrue(all(c[:4] == ["/proc/self/fd/123", "--exact", "--ignored", "--test-threads=1"] for c in calls))
        self.assertEqual(result["tests"], [{"name": n, "status": "PASS"} for n in archive.TESTS])
        self.assertEqual(result["cleanup"], "PASS")
        self.assertEqual(fixture.before_test.call_count, 16)
        self.assertEqual(fixture.after_test.call_count, 16)

    def test_nonzero_timeout_cancellation_and_callback_panic_never_pass(self):
        for failure, code in (("nonzero", "NAMED_TEST_RESULT_INVALID"), ("timeout", "NATIVE_TEST_TIMEOUT"),
                              ("cancel", "INTERRUPTED"), ("panic", "ARCHIVE_ROOT_UNEXPECTED_FAILURE")):
            status, result, calls, _ = self.run_driver(failure)
            self.assertEqual(status, 1)
            self.assertEqual(result["status"], "FAIL")
            self.assertEqual(result["error"], code)
            self.assertEqual(len(calls), 1)
            self.assertEqual(result["cleanup"], "PASS")  # Mocked independent empty-namespace check.

    def test_unreaped_child_cannot_emit_success_or_claim_cleanup(self):
        with self.assertRaisesRegex(RuntimeError, "mock unreaped"):
            self.run_driver("unreaped")
        self.assertEqual(self.last_output.getvalue(), "")
        self.last_fixture.cleanup.assert_not_called()
        projected = runner._root_failure_projection("", "a" * 40, "b" * 64)
        self.assertEqual(projected["cleanup"], "NOT_CONFIRMED")
        self.assertEqual(projected["root_result_status"], "UNKNOWN")


if __name__ == "__main__":
    unittest.main()
