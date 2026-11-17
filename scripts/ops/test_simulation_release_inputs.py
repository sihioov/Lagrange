#!/usr/bin/env python3
"""Local, non-root stdlib tests for scripts/ops/simulation-release-inputs.py.

Real: the installer validators (`validate_config`/`validate_binding`), the file
writes, modes and link counts.  Seamed: only root ownership admission
(`read_protected`, `root_directory`, `chown_root`, `ROOT_UID`).
"""
from __future__ import annotations

import contextlib
import copy
import hashlib
import importlib.util
import io
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
OPS = ROOT / "scripts/ops"


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


inputs = load("simulation_release_inputs_test_module", OPS / "simulation-release-inputs.py")
fixture_module = load("simulation_release_fixture_test_module",
                      OPS / "test_owner_beta_simulation_release.py")
release = inputs.release
daily = inputs.daily

COMMIT = fixture_module.COMMIT
PREVIOUS = "b2" * 20
OWNER = "22222222-2222-4222-8222-222222222222"
REFERENCE = "synthetic-previous-reference"
ENTITLEMENT = "sha256:" + "7" * 64
HISTORY = "/synthetic/previous-history"
LIMITS = {"memory_bytes": 4096, "nano_cpus": 5, "health_tmpfs_bytes": 6, "pids_limit": 7,
          "startup_timeout_seconds": 8, "health_max_age_seconds": 9,
          "stop_timeout_seconds": 20}
DAILY_FIXTURE = ROOT / "tests/fixtures/owner-beta-daily-install/config.json"
DAILY_COMMIT = "a" * 40


def fake_read_protected(path, gid, mode, maximum):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        info = os.fstat(fd)
        if (not stat.S_ISREG(info.st_mode) or info.st_nlink != 1
                or stat.S_IMODE(info.st_mode) != mode or info.st_size > maximum
                or info.st_uid != os.geteuid()):
            raise OSError("protected")
        return os.read(fd, maximum + 1)
    finally:
        os.close(fd)


@contextlib.contextmanager
def fake_root_directory(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        yield fd
    finally:
        os.close(fd)


def patches():
    stack = contextlib.ExitStack()
    stack.enter_context(mock.patch.object(inputs, "read_protected", fake_read_protected))
    stack.enter_context(mock.patch.object(inputs, "root_directory", fake_root_directory))
    stack.enter_context(mock.patch.object(inputs, "chown_root", lambda fd: None))
    stack.enter_context(mock.patch.object(inputs, "ROOT_UID", os.geteuid()))
    return stack


def run_cli(argv):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = inputs.main(argv)
    return code, out.getvalue(), err.getvalue()


def sign(receipt):
    data = json.dumps(receipt, separators=(",", ":")).encode("utf-8")
    return data, hashlib.sha256(data).hexdigest().encode("ascii") + b"\n"


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.fixture = fixture_module.binding_fixture()
        previous = copy.deepcopy(self.fixture["config"])
        previous.update(code_commit=PREVIOUS, owner=OWNER, entitlement_reference=REFERENCE,
                        entitlement_sha256=ENTITLEMENT, historical_root=HISTORY,
                        limits=dict(LIMITS), manifest_sha256="sha256:" + "8" * 64,
                        image_id="sha256:" + "9" * 64, binary_sha256="sha256:" + "a" * 64,
                        receipt_sha256="sha256:" + "b" * 64,
                        source_input_sha256="sha256:" + "c" * 64,
                        recipe_sha256="sha256:" + "d" * 64)
        self.previous = previous
        self.release = self.root / "releases" / PREVIOUS
        self.write_previous(previous)
        self.manifest = self.write("build/manifest", self.fixture["manifest"], 0o644)
        self.receipt = self.write("build/artifact.json", self.fixture["receipt_data"], 0o644)
        self.complete = self.write("build/COMPLETE", self.fixture["complete"], 0o644)
        (self.root / "current").symlink_to(self.release)
        self.out = self.root / "out"
        self.out.mkdir(mode=0o700)
        self.out.chmod(0o700)
        self.stack = patches()
        self.stack.__enter__()
        self.addCleanup(self.stack.close)

    def write(self, relative, data, mode):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.unlink(missing_ok=True)
        path.write_bytes(data)
        path.chmod(mode)
        return path

    def write_previous(self, config, daily_config=None):
        data = (json.dumps(config, indent=1) + "\n").encode()
        self.write(f"releases/{PREVIOUS}/.owner-beta-simulation/config.json", data, 0o440)
        if daily_config is not None:
            self.write(f"releases/{PREVIOUS}/.owner-beta-daily/config.json", daily_config, 0o440)

    def argv(self, mode="--apply", extra=(), **override):
        values = {"commit": COMMIT, "manifest": self.manifest, "receipt": self.receipt,
                  "complete": self.complete, "previous-release": self.release,
                  "output-dir": self.out}
        values.update(override)
        argv = [mode]
        for key, value in values.items():
            if value is not None:
                argv += ["--" + key, str(value)]
        return argv + list(extra)

    def assertFails(self, code, argv=None, cause=None):
        status, out, err = run_cli(self.argv() if argv is None else argv)
        self.assertEqual(status, 1, (out, err))
        self.assertEqual(out, "")
        self.assertIn(f"code={code}", err)
        if cause:
            self.assertIn(f"cause={cause}", err)
        self.assertEqual(sorted(os.listdir(self.out)), [])
        return err

    def reissue_receipt(self, mutate):
        receipt = copy.deepcopy(self.fixture["receipt"])
        mutate(receipt)
        data, complete = sign(receipt)
        self.receipt.write_bytes(data)
        self.complete.write_bytes(complete)


class SimulationInputsTest(Base):
    def test_apply_writes_exact_modes_and_validates(self):
        status, out, err = run_cli(self.argv())
        self.assertEqual((status, err), (0, ""))
        self.assertEqual(
            out, f"SIMULATION_RELEASE_INPUTS: PASS commit={COMMIT} files="
                 "simulation-config.json,simulation-artifact.json,simulation-COMPLETE\n")
        modes = {name: stat.S_IMODE(os.stat(self.out / name).st_mode)
                 for name in os.listdir(self.out)}
        self.assertEqual(modes, {"simulation-config.json": 0o600,
                                 "simulation-artifact.json": 0o444,
                                 "simulation-COMPLETE": 0o444})
        config = (self.out / "simulation-config.json").read_bytes()
        receipt = (self.out / "simulation-artifact.json").read_bytes()
        complete = (self.out / "simulation-COMPLETE").read_bytes()
        release.validate_binding(config, self.fixture["manifest"], receipt, complete, COMMIT)
        self.assertEqual(receipt, self.fixture["receipt_data"])
        self.assertEqual(complete, self.fixture["complete"])
        for name in os.listdir(self.out):  # copied, never linked
            self.assertEqual(os.stat(self.out / name).st_nlink, 1)
        self.assertNotEqual(os.stat(self.out / "simulation-artifact.json").st_ino,
                            os.stat(self.receipt).st_ino)
        self.assertTrue(config.endswith(b"\n"))
        expected = json.dumps(json.loads(config), ensure_ascii=False, sort_keys=True,
                              separators=(",", ":")) + "\n"
        self.assertEqual(config.decode(), expected)

    def test_only_listed_values_are_carried(self):
        built = json.loads(inputs.build_all(
            COMMIT, str(self.release), str(self.manifest), str(self.receipt),
            str(self.complete), False)[0]["simulation-config.json"][0])
        for key in ("owner", "entitlement_reference", "entitlement_sha256",
                    "historical_root", "limits"):
            self.assertEqual(built[key], self.previous[key])
        for key in ("code_commit", "manifest_sha256", "image_id", "receipt_sha256",
                    "binary_sha256", "source_input_sha256", "recipe_sha256"):
            self.assertNotEqual(built[key], self.previous.get(key))
        self.assertEqual(built["code_commit"], COMMIT)
        self.assertEqual(built["image_id"], self.fixture["images"][release.BIN])
        self.assertEqual(built["binary_sha256"], "sha256:" + self.fixture["receipt"]["binary_sha256"])

    def test_plan_touches_no_file_and_preflight_writes_nothing(self):
        argv = self.argv("--plan", manifest="/nonexistent/manifest")
        status, out, err = run_cli(argv)
        self.assertEqual((status, err), (0, ""))
        self.assertTrue(out.startswith(f"SIMULATION_RELEASE_INPUTS: PLAN commit={COMMIT} files="))
        status, out, err = run_cli(self.argv("--preflight"))
        self.assertEqual((status, err), (0, ""))
        self.assertIn("PREFLIGHT", out)
        self.assertEqual(os.listdir(self.out), [])

    def test_no_value_reaches_stdout_or_stderr(self):
        secrets = [OWNER, REFERENCE, ENTITLEMENT, HISTORY, str(self.root), "7" * 64,
                   self.fixture["receipt"]["binary_sha256"]]
        outputs = []
        for argv in (self.argv("--plan"), self.argv("--preflight"), self.argv()):
            status, out, err = run_cli(argv)
            self.assertEqual(status, 0)
            outputs += [out, err]
        # failure paths too: bad previous, non-empty output
        self.assertEqual(run_cli(self.argv())[0], 1)
        bad = copy.deepcopy(self.previous)
        bad["code_commit"] = COMMIT
        self.write_previous(bad)
        _, out, err = run_cli(self.argv(extra=()))
        outputs += [out, err]
        for text in outputs:
            for secret in secrets:
                self.assertNotIn(secret, text)

    def test_output_is_no_clobber(self):
        self.assertEqual(run_cli(self.argv())[0], 0)
        before = {n: (self.out / n).read_bytes() for n in os.listdir(self.out)}
        status, _, err = run_cli(self.argv())
        self.assertEqual(status, 1)
        self.assertIn("code=SIMULATION_INPUTS_OUTPUT", err)
        self.assertEqual({n: (self.out / n).read_bytes() for n in os.listdir(self.out)}, before)

    # ---- fail-closed codes -------------------------------------------------------

    def test_arguments(self):
        for argv in (
            self.argv(commit="not-a-commit"),
            self.argv(commit="0" * 40),
            self.argv(manifest="relative/manifest"),
            self.argv(**{"output-dir": "/tmp/../x"}),
            self.argv(**{"previous-release": "/synthetic/not-a-commit"}),
            self.argv(**{"previous-release": f"/synthetic/{COMMIT}"}),
            self.argv(manifest=None),
            self.argv(extra=["--plan"]),
            self.argv(extra=["--unknown"]),
            [],
        ):
            with self.subTest(argv=argv[:1] + argv[-2:]):
                self.assertFails("SIMULATION_INPUTS_ARGUMENTS", argv)

    def test_previous_invalid(self):
        wrong = copy.deepcopy(self.previous)
        wrong["code_commit"] = "c3" * 20
        self.write_previous(wrong)
        self.assertFails("SIMULATION_INPUTS_PREVIOUS", cause="SIMULATION_INSTALL_INVALID")
        extra = copy.deepcopy(self.previous)
        extra["extra"] = 1
        self.write_previous(extra)
        self.assertFails("SIMULATION_INPUTS_PREVIOUS")
        self.write_previous(self.previous)
        (self.release / ".owner-beta-simulation/config.json").chmod(0o444)
        self.assertFails("SIMULATION_INPUTS_PREVIOUS")
        (self.release / ".owner-beta-simulation/config.json").unlink()
        self.assertFails("SIMULATION_INPUTS_PREVIOUS")

    def test_previous_must_be_current_release(self):
        (self.root / "current").unlink()
        self.assertFails("SIMULATION_INPUTS_PREVIOUS_NOT_CURRENT")
        newer = self.root / "releases" / ("d4" * 20)
        newer.mkdir()
        (self.root / "current").symlink_to(newer)
        self.assertFails("SIMULATION_INPUTS_PREVIOUS_NOT_CURRENT")
        (self.root / "current").unlink()
        (self.root / "current").symlink_to(self.release)
        self.assertEqual(run_cli(self.argv("--preflight"))[0], 0)

    def test_manifest_invalid(self):
        v2, _ = fixture_module.manifest_bytes("LAGRANGE_RELEASE_MANIFEST_V2")
        self.manifest.write_bytes(v2)
        self.assertFails("SIMULATION_INPUTS_MANIFEST")
        self.manifest.write_bytes(self.fixture["manifest"].replace(COMMIT.encode(), b"c3" * 20))
        self.assertFails("SIMULATION_INPUTS_MANIFEST")
        self.manifest.write_bytes(self.fixture["manifest"].replace(b"\n", b"\r\n"))
        self.assertFails("SIMULATION_INPUTS_MANIFEST")
        self.manifest.unlink()
        self.assertFails("SIMULATION_INPUTS_MANIFEST")

    def test_receipt_invalid(self):
        self.complete.write_bytes(b"0" * 64 + b"\n")
        self.assertFails("SIMULATION_INPUTS_RECEIPT")
        self.complete.write_bytes(self.fixture["complete"])
        self.reissue_receipt(lambda r: r.update(source_commit="c3" * 20))
        self.assertFails("SIMULATION_INPUTS_RECEIPT")
        self.reissue_receipt(lambda r: r.update(binary_sha256="not-hex"))
        self.assertFails("SIMULATION_INPUTS_RECEIPT")
        self.receipt.write_bytes(b"{")
        self.assertFails("SIMULATION_INPUTS_RECEIPT")
        self.receipt.write_bytes(self.fixture["receipt_data"])
        self.complete.unlink()
        self.assertFails("SIMULATION_INPUTS_RECEIPT")

    def test_binding_rejected_by_installer_validator(self):
        self.reissue_receipt(lambda r: r["compile_env"].update(RUSTFLAGS="-C debuginfo=2"))
        self.assertFails("SIMULATION_INPUTS_BINDING", cause="SIMULATION_INSTALL_INVALID")
        self.reissue_receipt(lambda r: r.update(features=["x"]))
        self.assertFails("SIMULATION_INPUTS_BINDING")

    def test_output_directory_invalid(self):
        (self.out / "leftover").write_text("x")
        self.assertFails_keep("SIMULATION_INPUTS_OUTPUT")
        (self.out / "leftover").unlink()
        self.out.chmod(0o755)
        self.assertFails("SIMULATION_INPUTS_OUTPUT")
        self.out.chmod(0o700)
        self.assertFails("SIMULATION_INPUTS_OUTPUT", self.argv(**{"output-dir": self.root / "missing"}))
        with mock.patch.object(inputs, "ROOT_UID", os.geteuid() + 1):
            self.assertFails("SIMULATION_INPUTS_OUTPUT")
        self.assertFails("SIMULATION_INPUTS_OUTPUT", self.argv("--preflight", **{"output-dir": self.root / "missing"}))

    def assertFails_keep(self, code):
        status, out, err = run_cli(self.argv())
        self.assertEqual((status, out), (1, ""))
        self.assertIn(f"code={code}", err)

    def test_write_failure_removes_created_files(self):
        with mock.patch.object(inputs, "chown_root", side_effect=PermissionError):
            self.assertFails("SIMULATION_INPUTS_OUTPUT")

    def test_changed_after_write_is_rejected_and_cleaned(self):
        def tampering(path, gid, mode, maximum):
            data = fake_read_protected(path, gid, mode, maximum)
            return data + b" " if path.startswith(str(self.out)) and path.endswith("COMPLETE") else data
        with mock.patch.object(inputs, "read_protected", tampering):
            self.assertFails("SIMULATION_INPUTS_CHANGED")

    def test_real_reader_fails_closed_for_unprotected_files(self):
        if os.geteuid() == 0:
            self.skipTest("unprotected-file admission is only observable as non-root")
        self.stack.close()
        code, out, err = run_cli(self.argv())
        self.assertEqual((code, out), (1, ""))
        self.assertIn("code=SIMULATION_INPUTS_PREVIOUS", err)
        self.stack = patches()
        self.stack.__enter__()

    def test_cli_process_prints_only_closed_text(self):
        result = subprocess.run(
            [sys.executable, "-I", str(OPS / "simulation-release-inputs.py"), "--plan",
             *[str(a) for a in self.argv("--plan")[1:]]],
            capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stderr, "")
        self.assertNotIn(str(self.root), result.stdout)


class DailyInputsTest(Base):
    def setUp(self):
        super().setUp()
        self.daily_old = DAILY_FIXTURE.read_bytes().replace(
            DAILY_COMMIT.encode(), PREVIOUS.encode())
        self.write_previous(self.previous, self.daily_old)

    def test_daily_changes_only_code_commit(self):
        status, out, err = run_cli(self.argv(extra=["--daily"]))
        self.assertEqual((status, err), (0, ""))
        self.assertIn(",daily-config.json", out)
        path = self.out / "daily-config.json"
        self.assertEqual(stat.S_IMODE(os.stat(path).st_mode), 0o600)
        old, new = json.loads(self.daily_old), json.loads(path.read_bytes())
        self.assertEqual(new["code_commit"], COMMIT)
        self.assertEqual({k for k in old if old[k] != new[k]}, {"code_commit"})
        self.assertEqual(new["first_auto_session"], old["first_auto_session"])
        daily.validate_config(path.read_bytes(), COMMIT)
        daily.validate_config(self.daily_old, PREVIOUS)

    def test_daily_not_written_without_flag(self):
        self.assertEqual(run_cli(self.argv())[0], 0)
        self.assertNotIn("daily-config.json", os.listdir(self.out))

    def test_first_auto_session_change_is_refused(self):
        old = daily.validate_config(self.daily_old, PREVIOUS)
        new = dict(old, code_commit=COMMIT, first_auto_session="2026-09-01")
        with self.assertRaises(inputs.InputsError) as caught:
            inputs.assert_daily_only_commit_differs(old, new)
        self.assertEqual(caught.exception.code, "SIMULATION_INPUTS_DAILY_SESSION")
        other = dict(old, code_commit=COMMIT, max_cycle_seconds=61)
        with self.assertRaises(inputs.InputsError) as caught:
            inputs.assert_daily_only_commit_differs(old, other)
        self.assertEqual(caught.exception.code, "SIMULATION_INPUTS_DAILY")
        # a builder that drifted would be refused before any write
        with mock.patch.object(inputs.copy, "deepcopy", lambda v: dict(v, first_auto_session="2026-09-02")):
            with self.assertRaises(inputs.InputsError):
                inputs.build_daily(self.daily_old, PREVIOUS, COMMIT)

    def test_previous_daily_invalid(self):
        wrong = json.loads(self.daily_old)
        wrong["code_commit"] = "c3" * 20
        self.write_previous(self.previous, json.dumps(wrong).encode())
        self.assertFails("SIMULATION_INPUTS_DAILY", self.argv(extra=["--daily"]))
        self.write_previous(self.previous, b"{")
        self.assertFails("SIMULATION_INPUTS_DAILY", self.argv(extra=["--daily"]))
        (self.release / ".owner-beta-daily/config.json").unlink()
        self.assertFails("SIMULATION_INPUTS_DAILY", self.argv(extra=["--daily"]))

    def test_daily_output_is_value_free(self):
        _, out, err = run_cli(self.argv(extra=["--daily"]))
        for secret in ("00000000-0000-4000-8000-000000000001", "fixture-only", "2026-08-31",
                       str(self.root)):
            self.assertNotIn(secret, out + err)


if __name__ == "__main__":
    unittest.main()
