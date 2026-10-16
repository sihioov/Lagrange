#!/usr/bin/env python3
"""Disposable root-only SQL-render boundary tests. Never connects to a DB."""
from __future__ import annotations

import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "daily_release", ROOT / "scripts/ops/lib/owner-beta-daily-release.py")
assert SPEC and SPEC.loader
daily = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(daily)
COMMIT = "a" * 40


class DailyControlRenderTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if sys.platform != "linux" or os.geteuid() != 0:
            raise RuntimeError("DAILY_CONTROL_TEST_ENVIRONMENT")
        cls.anchor = Path(tempfile.mkdtemp(prefix="lagrange-daily-control-", dir="/var/lib"))
        cls.anchor.chmod(0o755)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.anchor)

    def setUp(self):
        self.root = Path(tempfile.mkdtemp(dir=self.anchor))
        self.root.chmod(0o755)
        self.release = self.root / "release"
        (self.release / "deploy/db").mkdir(mode=0o755, parents=True)
        self.template = self.release / "deploy/db/owner-beta-daily-control.sql"
        self.template.write_bytes((ROOT / "deploy/db/owner-beta-daily-control.sql").read_bytes())
        self.template.chmod(0o644)
        self.script = self.release / "scripts/ops/lib/owner-beta-daily-release.py"
        self.script.parent.mkdir(parents=True, mode=0o755)
        self.script.write_bytes((ROOT / "scripts/ops/lib/owner-beta-daily-release.py").read_bytes())
        self.script.chmod(0o644)
        self.source = self.root / "source.json"
        self.config = json.loads((ROOT / "tests/fixtures/owner-beta-daily-install/config.json").read_bytes())
        self.source.write_bytes(daily.canonical(self.config))
        self.source.chmod(0o600)
        self.policy = self.root / "policy.json"
        self.policy.write_bytes((ROOT / "configs/evidence/owner-beta-daily-price-policy-v1.json").read_bytes())
        self.policy.chmod(0o644)
        daily.stage_config(str(self.release), str(self.source), str(self.policy), COMMIT)

    def tearDown(self):
        shutil.rmtree(self.root)

    def render(self, action="prepare", capacity="10", lease="60", backoff="30"):
        return daily.render_control(str(self.release), COMMIT, action, capacity, lease, backoff)

    def test_render_uses_exact_installed_template_and_preserves_files(self):
        before = {str(p): p.read_bytes() for p in self.release.rglob("*") if p.is_file()}
        sql = self.render()
        self.assertNotIn("__ACTION__", sql)
        self.assertIn(daily.canonical(self.config).hex(), sql)
        self.assertIn(self.policy.read_bytes().hex(), sql)
        self.assertIn("'prepare',", sql)
        self.assertIn("'" + COMMIT + "', 10, 60, 30", sql)
        self.assertEqual(before, {str(p): p.read_bytes() for p in self.release.rglob("*") if p.is_file()})
        result = subprocess.run(
            [sys.executable, str(self.script),
             "render-control", "--release", str(self.release), "--commit", COMMIT,
             "--control-action", "prepare", "--capacity", "10", "--lease", "60", "--backoff", "30"],
            capture_output=True, timeout=10, check=False)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stderr, b"")
        self.assertEqual(result.stdout, sql.encode("ascii"))

    def test_closed_action_and_numeric_grammar_rejects_shell_or_sql_text(self):
        for action in ["", "reset", "enable;SELECT 1", "enable\n", "ENABLE"]:
            with self.subTest(action=action), self.assertRaises(daily.DailyInstallError):
                self.render(action=action)
        for value in [None, "", "0", "-1", "+1", "01", "1 ", " 1", "1;SELECT 1",
                      "1e3", "2_000", "9" * 20, True, 10]:
            with self.subTest(value=value), self.assertRaises(daily.DailyInstallError):
                self.render(capacity=value)
        for key, maximum in [("capacity", 2**31 - 1), ("lease", 2**31 - 1), ("backoff", 2**63 - 1)]:
            self.render(**{key: str(maximum)})
            with self.assertRaises(daily.DailyInstallError):
                self.render(**{key: str(maximum + 1)})

    def test_disable_accepts_no_capacity_and_never_resets_stored_values(self):
        sql = self.render("disable", None, None, None)
        self.assertIn("'" + COMMIT + "', NULL, NULL, NULL", sql)
        self.assertIn("SET enabled=false WHERE id='etf11'", sql)
        for values in [("10", None, None), (None, "60", None), (None, None, "30")]:
            with self.assertRaises(daily.DailyInstallError):
                self.render("disable", *values)

    def test_quote_and_unicode_values_are_only_hex_data(self):
        self.config["entitlement_reference"] = "quote' ; SELECT current_user; -- 한국어"
        target = self.release / daily.CONFIG_DIRECTORY / "config.json"
        target.chmod(0o640)
        target.write_bytes(daily.canonical(self.config))
        target.chmod(0o440)
        sql = self.render()
        self.assertNotIn(self.config["entitlement_reference"], sql)
        self.assertIn(daily.canonical(self.config).hex(), sql)
        self.assertEqual(bytes.fromhex(daily.canonical(self.config).hex()), target.read_bytes())

    def test_bad_commit_config_policy_and_template_fail_before_output(self):
        with self.assertRaises(daily.DailyInstallError):
            daily.render_control(str(self.release), "b" * 40, "prepare", "10", "60", "30")
        original = self.template.read_bytes()
        for body in [original.replace(b"__ACTION__", b"prepare"),
                     original + b"\n__ACTION__\n", original + b"\n__UNKNOWN__\n"]:
            self.template.write_bytes(body)
            with self.assertRaises(daily.DailyInstallError):
                self.render()
        self.template.write_bytes(original)
        target = self.release / daily.CONFIG_DIRECTORY / "policy.json"
        old = target.read_bytes()
        target.chmod(0o640)
        target.write_bytes(old + b" ")
        target.chmod(0o440)
        with self.assertRaises(daily.DailyInstallError):
            self.render()
        result = subprocess.run(
            [sys.executable, str(self.script),
             "render-control", "--release", str(self.release), "--commit", COMMIT,
             "--control-action", "prepare", "--capacity", "10", "--lease", "60", "--backoff", "30"],
            capture_output=True, timeout=10, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b"")
        self.assertEqual(result.stderr, b"DAILY_INSTALL_POLICY\n")

    def test_real_nonroot_principal_cannot_render_control(self):
        def demote():
            os.setgroups([])
            os.setgid(10003)
            os.setuid(10003)
        result = subprocess.run(
            [sys.executable, str(self.script),
             "render-control", "--release", str(self.release), "--commit", COMMIT,
             "--control-action", "disable"],
            capture_output=True, timeout=10, check=False, preexec_fn=demote)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b"")
        self.assertEqual(result.stderr, b"DAILY_INSTALL_PLATFORM\n")


if __name__ == "__main__":
    unittest.main(verbosity=2)
