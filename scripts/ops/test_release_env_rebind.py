#!/usr/bin/env python3
"""Local, non-root stdlib tests for scripts/ops/release-env-rebind.py.

The real strict dotenv parser (scripts/ops/lib/dotenv.sh) runs through bash.
Only the root-ownership trust predicate `trusted_file` is seamed, except in the
tests that exercise it as a non-root user to prove it fails closed.
"""
from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import os
import re
import shutil
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


rebind = load("release_env_rebind_test_module", OPS / "release-env-rebind.py")

PREVIOUS = "b2" * 20
COMMIT = "a1" * 20
OLD_RELEASE = "c3" * 20
OWNER = "33333333-3333-4333-8333-333333333333"
SUPPLY = {
    "schema": "r2-action-supply-v1", "enabled": True, "release_commit": PREVIOUS,
    "owner": OWNER, "policy_sha256": "sha256:" + "1" * 64, "activation_date": "2026-09-01",
    "base_candidate_sha256": "sha256:" + "2" * 64, "initial_civil_frontier": "2026-08-31",
    "entitlement_reference_sha256": "sha256:" + "3" * 64, "raw_owner_uid": 10001,
    "timeout_seconds": 120,
}


def encode(config):
    return json.dumps(config, separators=(",", ":")).encode().hex()


def env_text(supply=None, newline="\n", commit=PREVIOUS):
    supply = encode(SUPPLY) if supply is None else supply
    lines = [
        "# synthetic production env",
        "",
        "LAGRANGE_DATA_DIR=/opt/lagrange/data",
        f"LAGRANGE_CODE_COMMIT={commit}",
        "API_APP_ENV=production",
        f"RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX={supply}",
        "RUST_LOG=info",
    ]
    return newline.join(lines) + newline


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.env = self.root / "releases" / PREVIOUS / "deploy/compose/.env"
        self.env.parent.mkdir(parents=True)
        (self.root / "current").symlink_to(self.root / "releases" / PREVIOUS)
        self.out_dir = self.root / "out"
        self.out_dir.mkdir()
        self.output = self.out_dir / ".env"
        self.trust = mock.patch.object(rebind, "trusted_file", lambda path: True)
        self.trust.start()
        self.addCleanup(self.trust.stop)

    def put(self, text, raw=None):
        self.env.write_bytes(raw if raw is not None else text.encode())
        self.env.chmod(0o600)

    def argv(self, mode="--apply", **override):
        values = {"previous-env": self.env, "commit": COMMIT, "output": self.output}
        values.update(override)
        return [mode] + [x for k, v in values.items() if v is not None
                         for x in ("--" + k, str(v))]

    def run_cli(self, argv=None):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = rebind.main(self.argv() if argv is None else argv)
        return code, out.getvalue(), err.getvalue()

    def assertFails(self, code, argv=None):
        status, out, err = self.run_cli(argv)
        self.assertEqual((status, out), (1, ""), err)
        self.assertEqual(err, f"RELEASE_ENV_REBIND: FAIL code={code}\n")
        self.assertFalse(self.output.exists())


class TransformTest(Base):
    def test_only_two_lines_change(self):
        text = env_text()
        self.put(text)
        status, out, err = self.run_cli()
        self.assertEqual((status, err), (0, ""))
        self.assertEqual(out, f"RELEASE_ENV_REBIND: PASS commit={COMMIT} changed_keys=2\n")
        old = text.splitlines(keepends=True)
        new = self.output.read_bytes().decode().splitlines(keepends=True)
        self.assertEqual(len(old), len(new))
        changed = [i for i, (a, b) in enumerate(zip(old, new)) if a != b]
        self.assertEqual(len(changed), 2)
        self.assertEqual(new[changed[0]], f"LAGRANGE_CODE_COMMIT={COMMIT}\n")
        expected = dict(SUPPLY, release_commit=COMMIT)
        self.assertEqual(new[changed[1]],
                         "RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX=" + encode(expected) + "\n")
        self.assertEqual(list(json.loads(bytes.fromhex(new[changed[1]].split("=", 1)[1]))),
                         list(SUPPLY))  # key order preserved
        self.assertEqual(stat.S_IMODE(os.stat(self.output).st_mode), 0o600)
        self.assertEqual(os.stat(self.output).st_nlink, 1)
        self.assertEqual(os.listdir(self.out_dir), [".env"])  # no temp file left

    def test_crlf_and_missing_final_newline_preserved(self):
        self.put(env_text(newline="\r\n"))
        self.assertEqual(self.run_cli()[0], 0)
        raw = self.output.read_bytes()
        self.assertTrue(raw.endswith(b"\r\n"))
        for line in raw.split(b"\r\n")[:-1]:
            self.assertNotIn(b"\n", line)
        self.assertIn(f"LAGRANGE_CODE_COMMIT={COMMIT}\r\n".encode(), raw)
        self.output.unlink()
        self.put("", raw=env_text().rstrip("\n").encode())
        self.assertEqual(self.run_cli()[0], 0)
        self.assertFalse(self.output.read_bytes().endswith(b"\n"))

    def test_plan_and_preflight_write_nothing(self):
        self.put(env_text())
        status, out, err = self.run_cli(self.argv("--plan", **{"previous-env": self.env}))
        self.assertEqual((status, err), (0, ""))
        self.assertIn("PLAN", out)
        status, out, err = self.run_cli(self.argv("--preflight"))
        self.assertEqual((status, err), (0, ""))
        self.assertIn("PREFLIGHT", out)
        self.assertEqual(os.listdir(self.out_dir), [])

    def test_plan_does_not_read_files(self):
        argv = self.argv("--plan", **{"previous-env": self.env})
        self.assertFalse(self.env.exists())
        self.assertEqual(self.run_cli(argv)[0], 0)

    def test_no_value_reaches_stdout_or_stderr(self):
        text = env_text()
        self.put(text)
        hex_value = encode(SUPPLY)
        texts = []
        texts += self.run_cli(self.argv("--plan"))[1:]
        texts += self.run_cli(self.argv("--preflight"))[1:]
        texts += self.run_cli()[1:]
        texts += self.run_cli()[1:]  # output exists
        self.put(env_text(supply="zz"))
        texts += self.run_cli(self.argv(output=self.out_dir / "other"))[1:]
        self.put(env_text() + "BAD=$X\n")
        texts += self.run_cli(self.argv(output=self.out_dir / "other"))[1:]
        for text_out in texts:
            for secret in (OWNER, hex_value, encode(dict(SUPPLY, release_commit=COMMIT)),
                           "/opt/lagrange/data", "production", str(self.root), PREVIOUS):
                self.assertNotIn(secret, text_out)

    def test_real_dotenv_accepts_output(self):
        self.put(env_text())
        self.assertEqual(self.run_cli()[0], 0)
        self.assertTrue(rebind.dotenv_ok(str(self.output)))

    # ---- fail-closed codes -------------------------------------------------------

    def test_arguments(self):
        self.put(env_text())
        other = self.root / "releases" / "not-a-commit" / "deploy/compose/.env"
        for argv in (
            self.argv(commit="xyz"), self.argv(commit="0" * 40), self.argv(commit=PREVIOUS),
            self.argv(**{"previous-env": "relative/.env"}),
            self.argv(**{"previous-env": other}),
            self.argv(**{"previous-env": self.root / ".env"}),
            self.argv(output="relative"), self.argv(output=self.env),
            self.argv(output=str(self.out_dir) + "/../x"),
            self.argv(output=None), self.argv() + ["--plan"], [], self.argv() + ["--zzz"],
        ):
            with self.subTest(argv=argv[:1] + argv[-1:]):
                self.assertFails("ENV_REBIND_ARGUMENTS", argv)

    def test_source_trust_real_predicate(self):
        self.trust.stop()
        try:
            self.put(env_text())
            self.env.chmod(0o644)  # not 0600 for any owner
            self.assertFails("ENV_REBIND_SOURCE_TRUST")
            self.env.chmod(0o600)
            if os.geteuid() != 0:  # not root-owned
                self.assertFails("ENV_REBIND_SOURCE_TRUST")
            link = self.env.parent / "link"
            link.symlink_to(self.env)
            self.assertFalse(rebind.trusted_file(str(link)))
        finally:
            self.trust.start()

    def test_source_unreadable_or_irregular(self):
        self.assertFails("ENV_REBIND_SOURCE_TRUST")  # missing (trust seam open)
        self.env.mkdir()
        self.assertFails("ENV_REBIND_SOURCE_TRUST")

    def test_source_syntax(self):
        for bad in ("BAD=$HOME", 'BAD="q"', "BAD=a\\b", "BAD=a #c", "no equals", "1BAD=x",
                    "LAGRANGE_DATA_DIR=again"):
            with self.subTest(bad=bad):
                self.put(env_text() + bad + "\n")
                self.assertFails("ENV_REBIND_SOURCE_SYNTAX")
        self.put("", raw=env_text().encode() + b"BAD=\xff\xfe\n")
        self.assertEqual(self.run_cli()[0], 0)  # bytes are preserved; dotenv is byte-literal
        self.output.unlink()

    def test_commit_line(self):
        self.put(env_text().replace("LAGRANGE_CODE_COMMIT=" + PREVIOUS + "\n", ""))
        self.assertFails("ENV_REBIND_COMMIT_LINE")
        self.put(env_text(commit=OLD_RELEASE))  # not the release directory name
        self.assertFails("ENV_REBIND_COMMIT_LINE")
        self.put(env_text(commit="short"))
        self.assertFails("ENV_REBIND_COMMIT_LINE")
        self.put(env_text() + f"LAGRANGE_CODE_COMMIT={PREVIOUS}\n")  # duplicate: dotenv rejects
        self.assertFails("ENV_REBIND_SOURCE_SYNTAX")
        self.put(env_text(commit=PREVIOUS + " "))
        self.assertFails("ENV_REBIND_COMMIT_LINE")

    def test_supply_absent(self):
        self.put(env_text().replace("RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX=" + encode(SUPPLY) + "\n", ""))
        self.assertFails("ENV_REBIND_SUPPLY_ABSENT")

    def test_supply_shape(self):
        valid = encode(SUPPLY)
        cases = {
            "uppercase": valid.upper(),
            "odd": valid[:-1],
            "empty": "",
            "not-hex": "zz" * 8,
            "oversize": "20" * 4100,
            "not-json": b"not json".hex(),
            "array": b"[]".hex(),
            "duplicate-key": ('{"schema":"r2-action-supply-v1",' + '"schema":"r2-action-supply-v1"}').encode().hex(),
            "nan": json.dumps(dict(SUPPLY, timeout_seconds=float("nan"))).encode().hex(),
            "schema": encode(dict(SUPPLY, schema="r2-action-supply-v2")),
            "unknown-key": encode(dict(SUPPLY, extra=1)),
            "missing-key": encode({k: v for k, v in SUPPLY.items() if k != "owner"}),
            "bad-release-commit": encode(dict(SUPPLY, release_commit="short")),
            "numeric-release-commit": encode(dict(SUPPLY, release_commit=1)),
            "non-utf8": b"\xff\xfe".hex(),
        }
        for name, value in cases.items():
            with self.subTest(name=name):
                self.put(env_text(supply=value))
                self.assertFails("ENV_REBIND_SUPPLY_SHAPE")
        self.put(env_text() + f"RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX={valid}\n")
        self.assertFails("ENV_REBIND_SOURCE_SYNTAX")  # duplicate key

    def test_supply_line_count_guard(self):
        with self.assertRaises(rebind.RebindError) as caught:
            rebind.transform((env_text() + "RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX=00\n").encode(),
                             PREVIOUS, COMMIT)
        self.assertEqual(caught.exception.code, "ENV_REBIND_SUPPLY_SHAPE")

    def test_diff_self_check(self):
        old = env_text().encode()
        good = rebind.transform(old, PREVIOUS, COMMIT)
        rebind.self_check(old, good, COMMIT)
        supply_hex = encode(dict(SUPPLY, release_commit=COMMIT))
        mutations = {
            "extra-line": good + b"EXTRA=1\n",
            "other-key": good.replace(b"API_APP_ENV=production", b"API_APP_ENV=staging"),
            "commit-unchanged": good.replace(f"CODE_COMMIT={COMMIT}".encode(),
                                             f"CODE_COMMIT={PREVIOUS}".encode()),
            "supply-field": good.replace(
                supply_hex.encode(),
                encode(dict(SUPPLY, release_commit=COMMIT, timeout_seconds=121)).encode()),
            "supply-unchanged": good.replace(supply_hex.encode(), encode(SUPPLY).encode()),
            "comment": good.replace(b"# synthetic", b"# changed"),
            "removed-key": good.replace(b"RUST_LOG=info\n", b""),
        }
        for name, mutated in mutations.items():
            with self.subTest(name=name):
                with self.assertRaises(rebind.RebindError) as caught:
                    rebind.self_check(old, mutated, COMMIT)
                self.assertEqual(caught.exception.code, "ENV_REBIND_DIFF")

    def test_output_exists_is_not_clobbered(self):
        self.put(env_text())
        self.output.write_bytes(b"keep\n")
        for mode in ("--preflight", "--apply"):
            status, out, err = self.run_cli(self.argv(mode))
            self.assertEqual((status, out), (1, ""))
            self.assertEqual(err, "RELEASE_ENV_REBIND: FAIL code=ENV_REBIND_OUTPUT_EXISTS\n")
        self.assertEqual(self.output.read_bytes(), b"keep\n")
        self.output.unlink()
        self.output.symlink_to(self.root / "nowhere")  # dangling symlink still counts
        self.assertFails_exists()

    def assertFails_exists(self):
        status, _, err = self.run_cli()
        self.assertEqual(status, 1)
        self.assertIn("code=ENV_REBIND_OUTPUT_EXISTS", err)

    def test_link_race_is_not_clobbered(self):
        self.put(env_text())
        original = rebind.output_absent

        def racing(path):
            original(path)
            Path(path).write_bytes(b"raced\n")

        with mock.patch.object(rebind, "output_absent", racing):
            status, _, err = self.run_cli()
        self.assertEqual(status, 1)
        self.assertIn("code=ENV_REBIND_OUTPUT_EXISTS", err)
        self.assertEqual(self.output.read_bytes(), b"raced\n")
        self.assertEqual(sorted(os.listdir(self.out_dir)), [".env"])  # temp removed

    def test_output_trust(self):
        self.put(env_text())
        status, _, err = self.run_cli(self.argv(output=self.root / "missing-dir" / ".env"))
        self.assertEqual((status, err), (1, "RELEASE_ENV_REBIND: FAIL code=ENV_REBIND_OUTPUT_TRUST\n"))
        # post-write trust failure removes the file the tool itself created
        with mock.patch.object(rebind, "trusted_file", lambda path: path == str(self.env)):
            self.assertFails("ENV_REBIND_OUTPUT_TRUST")
        self.assertEqual(os.listdir(self.out_dir), [])
        # real predicate: not root:root (or not 0600 chain) means untrusted
        self.trust.stop()
        try:
            if os.geteuid() != 0:
                self.put(env_text())
                self.assertFalse(rebind.trusted_file(str(self.env)))
        finally:
            self.trust.start()

    def test_previous_must_be_current_release(self):
        self.put(env_text())
        (self.root / "current").unlink()
        self.assertFails("ENV_REBIND_PREVIOUS_NOT_CURRENT")  # no current link
        older = self.root / "releases" / OLD_RELEASE
        older.mkdir()
        (self.root / "current").symlink_to(older)  # current is another release
        self.assertFails("ENV_REBIND_PREVIOUS_NOT_CURRENT")
        (self.root / "current").unlink()
        (self.root / "current").symlink_to(self.root / "releases" / PREVIOUS)
        self.assertEqual(self.run_cli()[0], 0)

    def test_bash_ignores_caller_environment(self):
        marker = self.root / "side-effect"
        hook = self.root / "hook.sh"
        hook.write_text(f"touch '{marker}'\n")
        shim = self.root / "shim"
        shim.mkdir()
        for name in ("stat", "dirname"):  # trusted_file calls these through PATH
            (shim / name).write_text(f"#!/bin/sh\ntouch '{marker}'\nexec /usr/bin/{name} \"$@\"\n")
            (shim / name).chmod(0o755)
        self.put(env_text())
        hostile = {"BASH_ENV": str(hook), "ENV": str(hook), "PATH": f"{shim}:/usr/bin:/bin"}
        with mock.patch.dict(os.environ, hostile):
            self.trust.stop()
            try:
                rebind.trusted_file(str(self.env))
            finally:
                self.trust.start()
            self.assertTrue(rebind.dotenv_ok(str(self.env)))
        self.assertFalse(marker.exists())
        # control: the same hostile environment does run the shim when inherited
        subprocess.run(["bash", "-c", 'source "$1"; release_image_manifest_trusted_file "$2" x',
                        "_", str(rebind.MANIFEST_LIB), str(self.env)],
                       env=hostile, capture_output=True, check=False)
        self.assertTrue(marker.exists())

    def test_cross_language_supply_keys(self):
        source = (ROOT / "data-pipelines/collectors/src/action_supply.rs").read_text()
        body = re.search(r"struct Config \{(.*?)\n\}", source, re.S).group(1)
        fields = set(re.findall(r"^\s*(\w+):", body, re.M))
        self.assertEqual(fields, set(rebind.SUPPLY_KEYS))
        self.assertIn("deny_unknown_fields", source)
        self.assertIn('self.schema != "r2-action-supply-v1"', source)
        self.assertEqual(rebind.SUPPLY_SCHEMA, "r2-action-supply-v1")
        self.assertIn("value.len() > 8192", source)
        self.assertEqual(rebind.MAX_HEX, 8192)

    def test_cli_process_prints_only_closed_text(self):
        self.put(env_text())
        result = subprocess.run(
            [sys.executable, "-I", str(OPS / "release-env-rebind.py"), "--plan",
             "--previous-env", str(self.env), "--commit", COMMIT, "--output", str(self.output)],
            capture_output=True, text=True, check=False)
        self.assertEqual((result.returncode, result.stderr), (0, ""))
        self.assertEqual(result.stdout, f"RELEASE_ENV_REBIND: PLAN commit={COMMIT} changed_keys=2\n")


if __name__ == "__main__":
    unittest.main()
