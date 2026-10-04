#!/usr/bin/env python3
"""Synthetic orchestration tests; no Docker, root operation, or real WS state.

File type/mode/link checks and file operations are real. Only uid/gid metadata
is translated inside the disposable parent because these tests are unprivileged.
The fake container runs no Rust code; actual UID/mount/initializer acceptance
requires the separately reviewed disposable container gate.
"""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import types
import unittest
from unittest.mock import patch


SOURCE = Path(__file__).with_name("provision-owner-market-stream.py")
SPEC = importlib.util.spec_from_file_location("market_stream_provision_ops", SOURCE)
M = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = M
SPEC.loader.exec_module(M)
REVISION = "a" * 40
IMAGE_ID = "sha256:" + "b" * 64
SLOT = "00000000-0000-4000-8000-000000000001"


class FakeDocker:
    def __init__(self, case):
        self.case = case
        self.items = {}
        self.calls = []
        self.starts = []
        self.failure = None
        self.foreign = False
        self.running = False
        self.serial = 0

    def call(self, argv, timeout=10):
        self.case.assertEqual(argv[:3], M.DOCKER)
        args = argv[3:]
        self.calls.append(args)
        result = b""
        code = 0
        if args[:2] == ["image", "inspect"]:
            result = json.dumps([{"Id": IMAGE_ID, "Config": {
                "Labels": {"org.opencontainers.image.revision": REVISION},
                "User": "10001:10001", "Entrypoint": ["/usr/local/bin/owner-equity-v2-runner"]}}]).encode()
        elif args[0] == "ps":
            if "label=com.docker.compose.service=owner-equity-v2-runner" in args:
                result = b"e" * 64 + b"\n" if self.running else b""
            else:
                name = args[-1].removeprefix("name=^/").removesuffix("$")
                result = b"".join((i + "\n").encode() for i, x in self.items.items() if x["Name"] == "/" + name)
        elif args[0] == "create":
            self.serial += 1
            identifier = f"{self.serial:064x}"
            name = args[args.index("--name") + 1]
            token = args[args.index("--label") + 1].split("=", 1)[1]
            operation = args[args.index(IMAGE_ID) + 1]
            if operation == "initialize-new":
                record = json.loads((self.case.parent / M.RECORD).read_text())
                self.case.assertEqual(record["status"], "attempted")
                self.case.assertFalse((self.case.parent / M.STATE).exists())
            self.items[identifier] = {"Id": identifier, "Name": "/" + name,
                "Image": IMAGE_ID, "Config": {"Labels": {M.LABEL: "foreign" if self.foreign else token}},
                "State": {"Running": False, "ExitCode": 0}, "operation": operation}
            result = (identifier + "\n").encode()
        elif args[:2] == ["container", "inspect"]:
            if args[2] == "e" * 64:
                item = {"Id": args[2], "Config": {"Labels": {
                    "com.docker.compose.project": "lagrange-station",
                    "com.docker.compose.service": "owner-equity-v2-runner"}},
                    "State": {"Status": "running", "Running": True}}
            else:
                item = self.items[args[2]]
            result = json.dumps([item]).encode()
        elif args[0] == "start":
            item = self.items[args[-1]]
            self.starts.append(item["operation"])
            if item["operation"] == "initialize-new":
                self.case.synthetic_layout()
                if self.failure == "timeout":
                    item["State"]["Running"] = True
                    raise M.ProvisionError("ERR_PROCESS")
                if self.failure == "initializer":
                    code = 6
                    result = b"untrusted sentinel must not escape"
                else:
                    result = b"OK_INITIALIZED\n"
            else:
                result = b"OK_VALIDATED\n"
        elif args[0] == "rm":
            self.case.assertEqual(args[:2], ["rm", "--force"])
            del self.items[args[2]]
        else:
            self.case.fail("unexpected Docker operation: " + str(args[:2]))
        return subprocess.CompletedProcess(argv, code, result, b"")


class ProvisionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="lagrange-ws-provision-test-")
        self.addCleanup(self.tmp.cleanup)
        self.parent = Path(self.tmp.name) / "parent"
        self.parent.mkdir(mode=0o750)
        self.parent.chmod(0o750)
        self.ctx = M.Context(REVISION, SLOT, "17", self.parent, IMAGE_ID)
        self.fake = FakeDocker(self)
        real_fstat = os.fstat

        def translated(descriptor):
            actual = real_fstat(descriptor)
            path = Path(os.readlink(f"/proc/self/fd/{descriptor}"))
            if not path.is_relative_to(self.parent):
                return actual
            is_state = path.is_relative_to(self.parent / M.STATE)
            is_record = path.name.startswith(M.RECORD)
            fields = {x: getattr(actual, x) for x in (
                "st_mode", "st_dev", "st_ino", "st_nlink", "st_size", "st_mtime_ns", "st_ctime_ns")}
            fields.update(st_uid=10001 if is_state else 0, st_gid=0 if is_record else 10001)
            return types.SimpleNamespace(**fields)

        self.enterContext(patch.object(M.os, "fstat", side_effect=translated))
        self.enterContext(patch.object(M.os, "fchown"))
        self.enterContext(patch.object(M, "command", side_effect=self.fake.call))

    def synthetic_layout(self):
        state = self.parent / M.STATE
        anchors = self.parent / M.ANCHORS
        state.mkdir(mode=0o700)
        anchors.mkdir(mode=0o750)
        state.chmod(0o700)
        anchors.chmod(0o750)
        for name in ("connection.lock", "state.lock"):
            (anchors / name).touch(mode=0o440)
            (anchors / name).chmod(0o440)
        statefile = state / "approval-state-v1.json"
        statefile.write_bytes(b"synthetic bytes; not a validated Rust state\n")
        statefile.chmod(0o600)

    def snapshot(self):
        return {str(p.relative_to(self.parent)): (p.stat().st_ino, p.stat().st_mode,
                p.read_bytes() if p.is_file() else None) for p in self.parent.rglob("*")}

    def test_plan_has_no_installed_reads_or_processes(self):
        with patch.object(M, "installed_context") as context, contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(M.main(["--expected-commit", REVISION]), 0)
            context.assert_not_called()
        self.assertEqual(self.fake.calls, [])

    def test_arguments_pin_image_paths_users_caps_and_generation_without_healthcheck(self):
        for operation in ("initialize-new", "validate-existing"):
            argv = M.initializer_argv(self.ctx, operation, "name", "token")
            self.assertIn("--network=none", argv)
            self.assertIn("--read-only", argv)
            self.assertIn("--pull=never", argv)
            self.assertIn("--no-healthcheck", argv)
            self.assertEqual(argv[argv.index("--entrypoint") + 1], M.BIN)
            self.assertEqual(argv[argv.index("--user") + 1], "0:0" if operation == "initialize-new" else "10001:10001")
            mount = argv[argv.index("--mount") + 1]
            self.assertEqual(mount.endswith(",readonly"), operation == "validate-existing")
            self.assertEqual(argv[-6:], [IMAGE_ID, operation, "--credential-slot-id", SLOT,
                                        "--credential-generation", "17"])
            self.assertEqual([x for x in argv if x.startswith("--cap-add")],
                ["--cap-add=CHOWN", "--cap-add=FOWNER", "--cap-add=DAC_OVERRIDE"] if operation == "initialize-new" else [])
            self.assertFalse(any("secret" in x or "Approval" in x for x in argv))

    def test_first_initialization_is_recorded_before_start_and_reapply_is_read_only(self):
        M.execute(self.ctx, True)
        record = json.loads((self.parent / M.RECORD).read_text())
        self.assertEqual(record["status"], "initialized")
        self.assertEqual(self.fake.starts, ["initialize-new", "validate-existing"])
        before = self.snapshot()
        M.execute(self.ctx, False)
        self.assertEqual(self.snapshot(), before)
        self.assertEqual(self.fake.starts, ["initialize-new", "validate-existing", "validate-existing"])
        self.assertEqual(self.fake.items, {})

    def test_disappeared_domain_never_becomes_first_install_again(self):
        M.execute(self.ctx, True)
        shutil.rmtree(self.parent / M.STATE)
        shutil.rmtree(self.parent / M.ANCHORS)
        before = self.snapshot()
        with self.assertRaisesRegex(M.ProvisionError, "ERR_ALREADY_ATTEMPTED"):
            M.execute(self.ctx, True)
        with self.assertRaises(FileNotFoundError):
            M.execute(self.ctx, False)
        self.assertEqual(self.snapshot(), before)
        self.assertEqual(len(self.fake.starts), 2)

    def test_failed_and_timed_out_initialization_preserves_attempt_and_reaps_only_owned_container(self):
        for failure in ("initializer", "timeout"):
            with self.subTest(failure=failure):
                self.fake.failure = failure
                with self.assertRaises(M.ProvisionError):
                    M.execute(self.ctx, True)
                self.assertEqual(json.loads((self.parent / M.RECORD).read_text())["status"], "attempted")
                before = self.snapshot()
                with self.assertRaisesRegex(M.ProvisionError, "ERR_ALREADY_ATTEMPTED"):
                    M.execute(self.ctx, True)
                with self.assertRaisesRegex(M.ProvisionError, "ERR_RECORD"):
                    M.execute(self.ctx, False)
                self.assertEqual(self.snapshot(), before)
                self.assertEqual(self.fake.items, {})
                # Each subcase has its own disposable fixture; production has no reset API.
                for path in self.parent.iterdir():
                    shutil.rmtree(path) if path.is_dir() else path.unlink()

    def test_preexisting_or_symlink_leaf_denies_initialization_without_mutation(self):
        outside = Path(self.tmp.name) / "unrelated"
        outside.write_text("preserve")
        (self.parent / M.STATE).symlink_to(outside)
        with self.assertRaisesRegex(M.ProvisionError, "ERR_ALREADY_ATTEMPTED"):
            M.execute(self.ctx, True)
        self.assertEqual(outside.read_text(), "preserve")
        self.assertFalse((self.parent / M.RECORD).exists())
        self.assertEqual(self.fake.starts, [])

    def test_anchor_replacement_even_with_valid_metadata_is_not_blessed(self):
        M.execute(self.ctx, True)
        anchor = self.parent / M.ANCHORS / "connection.lock"
        original = anchor.with_name("held-original")
        anchor.rename(original)
        anchor.touch(mode=0o440)
        anchor.chmod(0o440)
        # Keep the original inode alive outside the domain to rule out inode reuse.
        original.rename(Path(self.tmp.name) / "held-original")
        before = self.snapshot()
        with self.assertRaisesRegex(M.ProvisionError, "ERR_LAYOUT_CHANGED"):
            M.execute(self.ctx, False)
        self.assertEqual(self.snapshot(), before)
        self.assertEqual(len(self.fake.starts), 2)

    def test_missing_empty_unsafe_or_symlink_state_denies_check_before_container(self):
        M.execute(self.ctx, True)
        statefile = self.parent / M.STATE / "approval-state-v1.json"
        for variant in ("empty", "writable", "symlink", "missing"):
            with self.subTest(variant=variant):
                statefile.unlink(missing_ok=True)
                if variant == "symlink":
                    statefile.symlink_to(Path(self.tmp.name) / "outside")
                elif variant != "missing":
                    statefile.write_bytes(b"" if variant == "empty" else b"value")
                    statefile.chmod(0o620 if variant == "writable" else 0o600)
                with self.assertRaises((M.ProvisionError, OSError)):
                    M.execute(self.ctx, False)
                self.assertEqual(len(self.fake.starts), 2)

    def test_fifo_and_hardlink_are_rejected_by_real_descriptor_metadata(self):
        fifo = self.parent / "fifo"
        os.mkfifo(fifo, 0o600)
        file = self.parent / "regular"
        file.touch(mode=0o600)
        os.link(file, self.parent / "hardlink")
        fd = os.open(self.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            for name in ("fifo", "hardlink"):
                with self.assertRaisesRegex(M.ProvisionError, "ERR_LAYOUT"):
                    M.metadata_at(fd, name, False, 0, 10001, 0o600)
        finally:
            os.close(fd)

    def test_fsync_failure_before_initializer_leaves_attempt_without_launch(self):
        with patch.object(M.os, "fsync", side_effect=OSError("synthetic fsync")):
            with self.assertRaises(OSError):
                M.execute(self.ctx, True)
        self.assertTrue((self.parent / M.RECORD).exists())
        self.assertEqual(self.fake.starts, [])

    def test_foreign_container_is_never_started_or_removed(self):
        self.fake.foreign = True
        with self.assertRaisesRegex(M.ProvisionError, "ERR_CLEANUP"):
            M.execute(self.ctx, True)
        self.assertEqual(self.fake.starts, [])
        self.assertFalse(any(x[0] == "rm" for x in self.fake.calls))
        self.assertEqual(len(self.fake.items), 1)

    def test_running_runner_denies_before_attempt(self):
        self.fake.running = True
        with self.assertRaisesRegex(M.ProvisionError, "ERR_RUNNER"):
            M.execute(self.ctx, True)
        self.assertFalse((self.parent / M.RECORD).exists())
        self.assertEqual(self.fake.starts, [])

    def test_nonroot_and_uninstalled_entry_are_denied_before_protected_reads(self):
        with patch.object(M.os, "getuid", return_value=1000):
            with self.assertRaisesRegex(M.ProvisionError, "ERR_ACTOR"):
                M.installed_context(REVISION, SOURCE)
        with patch.object(M.os, "getuid", return_value=0), patch.object(M.os, "geteuid", return_value=0):
            with self.assertRaisesRegex(M.ProvisionError, "ERR_RELEASE"):
                M.installed_context(REVISION, SOURCE)

    def test_installed_context_uses_real_shared_parsers_and_exact_release_binding(self):
        # Trust metadata is separately denied above and requires the real-root
        # fixture gate. Here the actual shared Bash parsers see only synthetic
        # env and complete manifest bytes; no Docker command is made.
        release_root = Path(self.tmp.name) / "installation"
        release = release_root / "releases" / REVISION
        ops = release / "scripts/ops"
        (ops / "lib").mkdir(parents=True)
        (release / "deploy/compose").mkdir(parents=True)
        entry = ops / SOURCE.name
        entry.write_bytes(SOURCE.read_bytes())
        for name in ("dotenv.sh", "release-image-manifest.sh"):
            shutil.copyfile(SOURCE.parent / "lib" / name, ops / "lib" / name)
        (release_root / "current").symlink_to("releases/" + REVISION)
        envfile = release / "deploy/compose/.env"
        values = {"LAGRANGE_CODE_COMMIT": REVISION, "OWNER_INTRADAY_QUOTES_MODE": "off",
            "KIS_READ_COORDINATION_MODE": "shared_required", "KIS_MARKET_STREAM_CREDENTIAL_SLOT_ID": SLOT,
            "KIS_READ_CREDENTIAL_GENERATION": "17", "LAGRANGE_RUNTIME_STATE_DIR": str(self.parent),
            "LAGRANGE_DATA_DIR": str(Path(self.tmp.name) / "data"),
            "LAGRANGE_ARTIFACTS_DIR": str(Path(self.tmp.name) / "artifacts"),
            "LAGRANGE_RUNTIME_SECRET_DIR": str(Path(self.tmp.name) / "runtime-secrets"),
            "LAGRANGE_SECRET_SOURCE_DIR": str(Path(self.tmp.name) / "secret-source")}
        services = ["db-role-bootstrap", "db-migrate", "api-server", "web", "research-worker",
            "recommendation-runner", "candidate-runner", "owner-beta-runner", "owner-equity-v2-runner",
            "nt-backtest-worker-1", "nt-backtest-worker-2", "paper-scheduler"]
        manifest = release / ".lagrange-release-manifest"
        valid_manifest = "LAGRANGE_RELEASE_MANIFEST_V2\ncommit|" + REVISION + "\n" + "".join(
            f"image|{s}|lagrange-station-{s}:{REVISION}|{IMAGE_ID}|{REVISION}\n" for s in services)
        manifest.write_text(valid_manifest)

        def actual_parser(argv, timeout=10):
            self.assertEqual(argv[0], "/bin/bash")
            return subprocess.run(argv, env=M.ENV, stdin=subprocess.DEVNULL, capture_output=True,
                                  check=False, timeout=timeout)

        # lstat of the one synthetic parent must carry the real-root fixture
        # metadata too; all other Path operations keep their real behavior.
        real_lstat = Path.lstat

        def parent_metadata(path):
            info = real_lstat(path)
            if path == self.parent:
                return types.SimpleNamespace(st_gid=10001, st_mode=info.st_mode)
            return info

        with patch.object(M, "trusted_directory"), patch.object(M, "trusted_file"), \
             patch.object(M.os, "getuid", return_value=0), patch.object(M.os, "geteuid", return_value=0), \
             patch.object(M, "command", side_effect=actual_parser), patch.object(Path, "lstat", parent_metadata):
            envfile.write_text("".join(f"{k}={v}\n" for k, v in values.items()))
            self.assertEqual(M.installed_context(REVISION, entry), self.ctx)
            for key, value in (("LAGRANGE_CODE_COMMIT", "c" * 40),
                               ("OWNER_INTRADAY_QUOTES_MODE", "owner_only"),
                               ("KIS_READ_CREDENTIAL_GENERATION", "017"),
                               ("LAGRANGE_RUNTIME_STATE_DIR", str(release / "state"))):
                with self.subTest(key=key):
                    bad = {**values, key: value}
                    envfile.write_text("".join(f"{k}={v}\n" for k, v in bad.items()))
                    with self.assertRaises(M.ProvisionError):
                        M.installed_context(REVISION, entry)
            envfile.write_text("".join(f"{k}={v}\n" for k, v in values.items()))
            manifest.write_text(valid_manifest.replace("image|web|", "image|unknown|"))
            with self.assertRaises(M.ProvisionError):
                M.installed_context(REVISION, entry)

    def test_closed_error_does_not_print_process_payload(self):
        self.fake.failure = "initializer"
        stderr = io.StringIO()
        with patch.object(M, "installed_context", return_value=self.ctx), contextlib.redirect_stderr(stderr):
            self.assertEqual(M.main(["--initialize-new", "--expected-commit", REVISION]), 1)
        self.assertEqual(stderr.getvalue(), "ERR_PROCESS\n")


if __name__ == "__main__":
    unittest.main()
