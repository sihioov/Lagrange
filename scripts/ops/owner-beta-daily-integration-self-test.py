#!/usr/bin/env python3
"""Root Daily installer mechanics in an isolated disposable hosted fixture."""
from __future__ import annotations

import fcntl
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SERVICES = (
    "db-role-bootstrap", "db-migrate", "api-server", "web", "research-worker",
    "recommendation-runner", "candidate-runner", "owner-beta-runner",
    "owner-equity-v2-runner", "nt-backtest-worker-1", "nt-backtest-worker-2",
    "paper-scheduler",
)
FILES = (
    "scripts/ops/deploy-production-release.sh",
    "scripts/ops/lib/release-image-manifest.sh",
    "scripts/ops/lib/dotenv.sh",
    "scripts/ops/lib/owner-beta-daily-release.sh",
    "scripts/ops/lib/owner-beta-daily-release.py",
    "scripts/ops/lib/owner-beta-daily-operations.py",
    "scripts/ops/lib/owner-beta-daily-base.py",
    "scripts/ops/lib/owner-beta-daily-runtime.py",
    "scripts/ops/lib/owner-beta-daily-backup-files.py",
    "scripts/ops/lib/service-status-policy.py",
    "scripts/ops/lib/owner-beta-simulation-release.py",
    "scripts/ops/lib/owner-beta-simulation-runtime.py",
    "scripts/ops/lib/owner-beta-simulation-operations.py",
    "configs/evidence/owner-beta-daily-price-policy-v1.json",
    "configs/evidence/owner-beta-daily-reader-v1.json",
    "configs/evidence/owner-beta-simulation-reader-v1.json",
)

class DailyReleaseIntegrationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if sys.platform != "linux" or os.geteuid() != 0:
            raise RuntimeError("DAILY_INSTALL_INTEGRATION_ENVIRONMENT_ERROR")
        cls.anchor = Path(tempfile.mkdtemp(prefix="lagrange-daily-integration-", dir="/var/lib"))
        cls.anchor.chmod(0o755)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.anchor)

    def setUp(self):
        self.root = Path(tempfile.mkdtemp(dir=self.anchor))
        self.root.chmod(0o755)
        self.repo = self.root / "repo"
        self.repo.mkdir(mode=0o755)
        self.install = self.root / "install"
        for name in FILES:
            target = self.repo / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
            target.chmod(0o644)
        # This suite has no DB or Docker daemon. Keep its historical Daily
        # installer assertions scoped to a synthetic no-simulation inventory;
        # the real controller and fail-closed inventory have separate tests.
        simulation = self.repo / "scripts/ops/lib/owner-beta-simulation-operations.py"
        simulation.write_text(
            "import os, sys\n"
            "args = sys.argv[1:]\n"
            "if len(args) != 11 or args[0] != 'transition' or args[1] != '--install-root' "
            "or args[3] != '--target-release' or args[5] != '--commit' "
            "or args[7] != '--lock-fd' or args[9] != '--action' or args[10] != 'activate':\n"
            "    raise SystemExit(1)\n"
            "root, target = args[2], args[4]\n"
            "if os.path.lexists(root + '/.owner-beta-simulation-reader-floor') "
            "or os.path.lexists(target + '/.owner-beta-simulation') "
            "or os.path.lexists(target + '/.owner-beta-simulation-receipt'):\n"
            "    raise SystemExit(1)\n"
            "print('SIMULATION_OPERATION: LEGACY_OFF_ACTIVATED')\n")
        compose = self.repo / "deploy/compose"
        compose.mkdir(parents=True)
        (compose / "compose.yml").write_text("services: {}\n")
        self.git("init", "-q")
        self.git("config", "user.name", "isolated fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.commit = self.commit_repo("initial")
        self.env = self.root / "release.env"
        self.manifest = self.root / "manifest"
        self.config = self.root / "daily.json"
        self.environment = {
            "PATH": os.environ["PATH"], "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "LC_ALL": "C", "LANG": "C", "PYTHONDONTWRITEBYTECODE": "1",
        }
        self.inputs("disabled")

    def tearDown(self):
        shutil.rmtree(self.root)

    def git(self, *args):
        return subprocess.check_output(
            ["git", "-c", "core.hooksPath=/dev/null", "-C", str(self.repo), *args],
            stderr=subprocess.PIPE, text=True, timeout=15).strip()

    def commit_repo(self, message):
        self.git("add", ".")
        self.git("commit", "-qm", message)
        return self.git("rev-parse", "HEAD")

    def inputs(self, mode):
        self.env.write_text(
            f"LAGRANGE_CODE_COMMIT={self.commit}\nLAGRANGE_DATA_DIR={self.root}/data\n"
            "OWNER_BETA_ACCESS_MODE=owner_only\nOWNER_BETA_PRICE_INPUT_MODE=sealed_v1\n"
            f"OWNER_BETA_PAPER_MODE=disabled\nOWNER_BETA_DAILY_MODE={mode}\n")
        self.env.chmod(0o600)
        rows = ["LAGRANGE_RELEASE_MANIFEST_V2", "commit|" + self.commit]
        for n, service in enumerate(SERVICES, 1):
            rows.append(f"image|{service}|lagrange-station-{service}:{self.commit}|sha256:{n:064x}|{self.commit}")
        self.manifest.write_text("\n".join(rows) + "\n")
        self.manifest.chmod(0o600)
        value = json.loads((ROOT / "tests/fixtures/owner-beta-daily-install/config.json").read_bytes())
        value["code_commit"] = self.commit
        self.config.write_text(json.dumps(value, separators=(",", ":")))
        self.config.chmod(0o600)

    def command(self, mode="--apply", *, commit=None, config=False, extra=()):
        args = ["bash", str(self.repo / "scripts/ops/deploy-production-release.sh"),
                mode, "--commit", commit or self.commit, "--install-root", str(self.install)]
        if mode == "--apply":
            args += ["--env-source", str(self.env), "--release-manifest", str(self.manifest)]
        if config:
            args += ["--daily-config-source", str(self.config)]
        args += list(extra)
        return subprocess.run(args, env=self.environment, capture_output=True, timeout=20)

    def expect_pass(self, result):
        self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8", "replace"))

    def expect_fail(self, result):
        self.assertNotEqual(result.returncode, 0)

    def current(self):
        return os.readlink(self.install / "current")

    def new_commit(self, mode):
        marker = self.repo / "fixture-revision"
        marker.write_text(str(self.commit))
        self.commit = self.commit_repo("next")
        self.inputs(mode)

    def test_legacy_off_apply_and_check_preserve_default_behavior(self):
        self.expect_pass(self.command())
        self.assertEqual(self.current(), "releases/" + self.commit)
        self.assertFalse((self.install / ".owner-beta-daily-reader-floor").exists())
        release = self.install / "releases" / self.commit
        self.assertFalse((release / ".owner-beta-daily").exists())
        before = {str(p.relative_to(self.install)): (p.lstat().st_mode, p.lstat().st_ino)
                  for p in self.install.rglob("*")}
        self.expect_pass(self.command("--check"))
        after = {str(p.relative_to(self.install)): (p.lstat().st_mode, p.lstat().st_ino)
                 for p in self.install.rglob("*")}
        self.assertEqual(before, after)

    def test_on_requires_separate_config_before_any_current_link(self):
        self.inputs("owner_only")
        self.expect_fail(self.command())
        self.assertFalse((self.install / "current").exists())
        self.assertFalse((self.install / ".owner-beta-daily-reader-floor").exists())

    def test_on_install_binds_exact_config_and_reserves_reader_floor(self):
        self.inputs("owner_only")
        before = self.config.read_bytes()
        self.expect_pass(self.command(config=True))
        target = self.install / "releases" / self.commit / ".owner-beta-daily/config.json"
        self.assertEqual(target.read_bytes(), before)
        info = target.stat()
        self.assertEqual((info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode)), (0, 10001, 0o440))
        floor = self.install / ".owner-beta-daily-reader-floor"
        self.assertEqual((floor.stat().st_uid, floor.stat().st_gid, stat.S_IMODE(floor.stat().st_mode)),
                         (0, 0, 0o600))
        self.expect_pass(self.command("--check"))
        self.assertEqual(self.config.read_bytes(), before)

    def test_off_preparation_keeps_config_without_activating_floor(self):
        self.expect_pass(self.command(config=True))
        self.assertFalse((self.install / ".owner-beta-daily-reader-floor").exists())
        target = self.install / "releases" / self.commit / ".owner-beta-daily/config.json"
        self.assertEqual(target.read_bytes(), self.config.read_bytes())
        self.expect_pass(self.command("--check"))

    def test_wrong_commit_and_untrusted_config_preserve_current(self):
        self.expect_pass(self.command())
        old = self.current()
        self.new_commit("owner_only")
        original = self.config.read_bytes()
        value = json.loads(original)
        value["code_commit"] = "f" * 40
        self.config.write_text(json.dumps(value, separators=(",", ":")))
        self.expect_fail(self.command(config=True))
        self.assertEqual(self.current(), old)
        self.config.write_bytes(original)
        self.config.chmod(0o640)
        self.expect_fail(self.command(config=True))
        self.assertEqual(self.current(), old)
        self.assertFalse((self.install / ".owner-beta-daily-reader-floor").exists())

    def test_old_reader_off_rollback_is_blocked_before_link_change(self):
        reader = self.repo / "configs/evidence/owner-beta-daily-reader-v1.json"
        reader_bytes = reader.read_bytes()
        reader.unlink()
        self.commit = self.commit_repo("old reader")
        self.inputs("disabled")
        old = self.commit
        self.expect_pass(self.command())
        reader.write_bytes(reader_bytes)
        self.commit = self.commit_repo("compatible reader")
        self.inputs("owner_only")
        self.expect_pass(self.command(config=True))
        current = self.current()
        floor = (self.install / ".owner-beta-daily-reader-floor").read_bytes()
        self.expect_fail(self.command("--rollback", commit=old))
        self.assertEqual(self.current(), current)
        self.assertEqual((self.install / ".owner-beta-daily-reader-floor").read_bytes(), floor)

    def test_compatible_off_rollback_keeps_reader_floor_and_history(self):
        self.expect_pass(self.command())
        old = self.commit
        history = self.install / "daily-history-fixture"
        history.write_bytes(b"do-not-delete")
        self.new_commit("owner_only")
        self.expect_pass(self.command(config=True))
        floor = (self.install / ".owner-beta-daily-reader-floor").read_bytes()
        self.expect_pass(self.command("--rollback", commit=old))
        self.assertEqual(self.current(), "releases/" + old)
        self.assertEqual(history.read_bytes(), b"do-not-delete")
        self.assertEqual((self.install / ".owner-beta-daily-reader-floor").read_bytes(), floor)

    def test_check_is_read_only_when_floor_is_missing(self):
        self.inputs("owner_only")
        self.expect_pass(self.command(config=True))
        # Deliberately remove only fixture state to exercise read-only check;
        # this is not an operational recovery or an allowed floor reset.
        floor = self.install / ".owner-beta-daily-reader-floor"
        floor.unlink()
        self.expect_pass(self.command("--check"))
        self.assertFalse(floor.exists())

    def test_held_release_lock_fails_without_changing_current(self):
        self.expect_pass(self.command())
        current = self.current()
        self.new_commit("disabled")
        with (self.install / ".lagrange-release.lock").open("r+b") as lock:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.expect_fail(self.command())
        self.assertEqual(self.current(), current)
        self.assertFalse((self.install / "releases" / self.commit).exists())

    def test_external_config_is_rejected_for_check_and_rollback(self):
        self.expect_pass(self.command())
        current = self.current()
        for mode in ("--check", "--rollback"):
            self.expect_fail(self.command(mode, config=True))
            self.assertEqual(self.current(), current)

    def test_protected_daily_mode_rejects_aliases_and_shell_inputs(self):
        script = (
            'source "$1/scripts/ops/lib/dotenv.sh"\n'
            'source "$1/scripts/ops/lib/owner-beta-daily-release.sh"\n'
            'dotenv_load "$2" || exit 1\n'
            'daily_release_parse_mode || { printf "%s\\n" "$DAILY_RELEASE_ERROR"; exit 1; }\n'
            'printf "%s\\n" "$DAILY_RELEASE_MODE"\n')
        cases = [
            ("", {}, "disabled"),
            ("OWNER_BETA_DAILY_MODE=owner_only\nOWNER_BETA_ACCESS_MODE=owner_only\n"
             "OWNER_BETA_PRICE_INPUT_MODE=sealed_v1\n", {}, "owner_only"),
            ("OWNER_BETA_DAILY_MODE=\n", {}, None),
            ("OWNER_BETA_DAILY_MODE=enabled\n", {}, None),
            ("OWNER_BETA_DAILY_MODE=owner_only\n", {}, None),
            ("", {"OWNER_BETA_DAILY_MODE": "owner_only"}, None),
            ("OWNER_BETA_DAILY_MODE_FILE=private\n", {}, None),
            ("", {"PRICE_BETA_DAILY_CONFIG_FILE": "/private"}, None),
            ("LAGRANGE_DAILY_VERIFIED_CONFIG_SHA256=sha256:" + "a" * 64 + "\n", {}, None),
        ]
        for index, (body, overlay, expected) in enumerate(cases):
            with self.subTest(case=index):
                self.env.write_text(body)
                result = subprocess.run(
                    ["bash", "-c", script, "fixture", str(self.repo), str(self.env)],
                    env={**self.environment, **overlay}, capture_output=True, timeout=5)
                if expected:
                    self.expect_pass(result)
                    self.assertEqual(result.stdout, (expected + "\n").encode())
                else:
                    self.expect_fail(result)

    def test_manifest_aliases_reuse_only_canonical_runner_image(self):
        script = (
            'source "$1/scripts/ops/lib/release-image-manifest.sh"\n'
            'release_image_manifest_load "$2" "$3" || exit 1\n'
            'release_image_manifest_write_compose_override "$4" "$5"\n')
        destination = self.root / "override"
        for mode, count in (("disabled", 12), ("owner_only", 14)):
            result = subprocess.run(
                ["bash", "-c", script, "fixture", str(self.repo), str(self.manifest),
                 self.commit, str(destination), mode], env=self.environment,
                capture_output=True, timeout=5)
            self.expect_pass(result)
            lines = destination.read_text().splitlines()
            names = [x.strip()[:-1] for x in lines if x.startswith("  ") and not x.startswith("   ")]
            self.assertEqual(len(names), count)
            self.assertEqual(names[:12], list(SERVICES))
            if mode == "owner_only":
                self.assertEqual(names[12:], ["owner-beta-daily-producer", "owner-beta-daily-admitter"])
                for alias in names[12:]:
                    index = lines.index("  " + alias + ":")
                    self.assertEqual(lines[index + 1], "    image: sha256:" + f"{8:064x}")
                    self.assertEqual(lines[index + 2], "    build: !reset null")
        before = destination.read_bytes()
        rejected = subprocess.run(
            ["bash", "-c", script, "fixture", str(self.repo), str(self.manifest),
             self.commit, str(destination), "unknown"], env=self.environment,
            capture_output=True, timeout=5)
        self.expect_fail(rejected)
        self.assertEqual(destination.read_bytes(), before)


class DailySupplierConfigTests(unittest.TestCase):
    """Real validator at both supplier boundaries; stop before any DB/provider use."""
    def setUp(self):
        if sys.platform != "linux" or os.geteuid() != 0:
            raise RuntimeError("DAILY_SUPPLIER_ACTUAL_ROOT_REQUIRED")
        self.directory = Path(tempfile.mkdtemp(prefix="lagrange-daily-supplier-", dir="/var/lib"))
        self.addCleanup(shutil.rmtree, self.directory)
        self.ops = self.directory / "repo/scripts/ops"
        self.ops.mkdir(parents=True)
        for name in ("validate-production-config.sh", "kis-daily-production.sh", "backfill-production.sh",
                     "lib/dotenv.sh", "lib/db.sh", "lib/kis-read-compose.sh", "lib/owner-beta-daily-release.sh"):
            target = self.ops / name
            target.parent.mkdir(exist_ok=True)
            shutil.copyfile(ROOT / "scripts/ops" / name, target)
        self.source = self.directory / "source"
        self.runtime = self.directory / "runtime"
        self.source.mkdir()
        db = ("postgres_password", "db_migration_owner_password", "db_app_password", "db_worker_password",
              "db_audit_password", "db_research_password", "db_admin_password", "db_daily_input_admitter_password")
        for name in (*db, "kis_app_key", "kis_app_secret"):
            path = self.source / name
            path.write_text("synthetic-distinct-" + name)
            path.chmod(0o400)
        specs = [("db-role-bootstrap", name, 999, 0o400) for name in db]
        specs += [("db-migrate", "db_migration_owner_password", 999, 0o400),
                  ("postgres", "postgres_password", 999, 0o440),
                  ("research-schema-check", "postgres_password", 999, 0o440)]
        specs += [("research-worker", name, 10001, 0o440)
                  for name in ("db_research_password", "kis_app_key", "kis_app_secret")]
        for service, name, uid, mode in specs:
            path = self.runtime / service / name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(self.source / name, path)
            os.chown(path, uid, uid)
            path.chmod(mode)
        self.values = {
            "LAGRANGE_CODE_COMMIT": "a" * 40, "LAGRANGE_DATA_DIR": str(self.directory / "data"),
            "LAGRANGE_SECRET_SOURCE_DIR": str(self.source), "LAGRANGE_RUNTIME_SECRET_DIR": str(self.runtime),
            "POSTGRES_USER": "postgres", "POSTGRES_DB": "lagrange", "RESEARCH_APP_ENV": "production",
            "RESEARCH_FETCH_MODE": "credentialed", "RESEARCH_ENTITLEMENT_REFERENCE": "synthetic-owned-fixture",
            "RESEARCH_CANDIDATE_ENABLED": "false", "OWNER_BETA_ACCESS_MODE": "owner_only",
            "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1", "OWNER_BETA_PAPER_MODE": "disabled",
            "OWNER_BETA_DAILY_MODE": "owner_only",
        }
        self.env = self.directory / "production.env"
        self.environment = {"PATH": "/usr/bin:/bin", "LC_ALL": "C", "LANG": "C",
            "LAGRANGE_ENV_FILE": str(self.env), "LAGRANGE_XKRX_CALENDAR_DIR": str(self.directory / "calendar"),
            "KIS_DAILY_STATE_DIR": str(self.directory / "state"),
            "KIS_DAILY_LOCK_FILE": str(self.directory / "state/daily.lock"),
            "BACKFILL_CONFIRM_EXTERNAL": "I_UNDERSTAND_READ_ONLY_KIS_CALLS"}
        self.write_env()

    def write_env(self):
        self.env.write_text("".join(key + "=" + value + "\n" for key, value in self.values.items()))
        self.env.chmod(0o600)

    def command(self, name, *args, extra=None):
        return subprocess.run(["bash", str(self.ops / name), *args],
            env={**self.environment, **(extra or {})}, capture_output=True, timeout=20)

    def test_on_environment_passes_real_supplier_validation(self):
        result = self.command("validate-production-config.sh", "--scope", "backfill")
        self.assertEqual(result.returncode, 0)
        self.assertIn(b"PRODUCTION_CONFIG: PASS (scope=backfill", result.stdout)
        del self.values["OWNER_BETA_DAILY_MODE"]
        self.write_env()
        self.assertEqual(self.command("validate-production-config.sh", "--scope", "backfill").returncode, 0)

    def prepare_range_raw_secrets(self):
        for name in ("kis_app_key", "kis_app_secret"):
            path = self.runtime / "research-range-raw" / name
            path.parent.mkdir(exist_ok=True)
            shutil.copyfile(self.source / name, path)
            os.chown(path, 10001, 10001)
            path.chmod(0o440)

    def test_on_environment_allows_database_free_manual_action_range_supplier(self):
        self.prepare_range_raw_secrets()
        # This operator path needs only the KIS Raw credentials. Do not require
        # or silently grant a database role merely because Daily is configured.
        for path in self.source.iterdir():
            if path.name not in ("kis_app_key", "kis_app_secret"):
                path.unlink()
        for path in self.runtime.iterdir():
            if path.name != "research-range-raw":
                shutil.rmtree(path)
        self.values.pop("POSTGRES_USER")
        self.values.pop("POSTGRES_DB")
        self.write_env()
        result = self.command("validate-production-config.sh", "--scope", "range-raw")
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        self.assertIn(b"PRODUCTION_CONFIG: PASS (scope=range-raw", result.stdout)
        self.assertNotEqual(self.command("validate-production-config.sh", "--scope", "backfill").returncode, 0)
        del self.values["OWNER_BETA_DAILY_MODE"]
        self.write_env()
        self.assertEqual(self.command("validate-production-config.sh", "--scope", "range-raw").returncode, 0)

    def test_manual_action_range_keeps_daily_mode_and_scope_boundaries(self):
        self.prepare_range_raw_secrets()
        self.assertEqual(self.command("validate-production-config.sh", "--scope", "range-raw").returncode, 0)
        for scope in ("infrastructure", "serving-prereqs", "range-raw-recovery"):
            with self.subTest(scope=scope):
                result = self.command("validate-production-config.sh", "--scope", scope)
                self.assertEqual(result.returncode, 1)
                self.assertIn(b"owner_beta_daily_requires_release_or_supplier_scope", result.stderr)
        for key, value in (("OWNER_BETA_ACCESS_MODE", "disabled"),
                           ("OWNER_BETA_PRICE_INPUT_MODE", "disabled"),
                           ("OWNER_BETA_PAPER_MODE", "enabled"),
                           ("PRICE_BETA_DAILY_CONFIG_SHA256", "sha256:" + "b" * 64)):
            with self.subTest(key=key):
                old = self.values.get(key)
                self.values[key] = value
                self.write_env()
                self.assertEqual(self.command("validate-production-config.sh", "--scope", "range-raw").returncode, 1)
                if old is None:
                    del self.values[key]
                else:
                    self.values[key] = old
        self.write_env()
        self.assertEqual(self.command("validate-production-config.sh", "--scope", "range-raw",
            extra={"OWNER_BETA_DAILY_MODE": "disabled"}).returncode, 1)
        key = self.runtime / "research-range-raw/kis_app_key"
        key.chmod(0o644)
        self.assertEqual(self.command("validate-production-config.sh", "--scope", "range-raw").returncode, 1)
        key.unlink()
        self.assertEqual(self.command("validate-production-config.sh", "--scope", "range-raw").returncode, 2)

    def test_daily_and_backfill_reach_real_validator_with_on_environment(self):
        # Keep the real validator and its arguments. Exit only AFTER its success
        # to prove composition without invoking the subsequent DB/provider path.
        real = self.ops / "validate-production-config.real.sh"
        (self.ops / "validate-production-config.sh").rename(real)
        (self.ops / "validate-production-config.sh").write_text(
            '#!/usr/bin/env bash\nset -euo pipefail\n'
            'bash "$(dirname "${BASH_SOURCE[0]}")/validate-production-config.real.sh" "$@"\n'
            'printf "SUPPLIER_REAL_VALIDATOR_REACHED\\n"\nexit 73\n')
        metadata = {"schema": "xkrx-historical-session-selection-v1",
            "requested_range": {"start": "2026-08-31", "end": "2026-08-31"},
            "artifact_range": {"start": "2026-08-31", "end": "2026-08-31"},
            "calendar_id": "synthetic-calendar-v1", "artifact_sha256": "sha256:" + "a" * 64,
            "artifact_size_bytes": 1, "session_count": 1, "skipped_non_session_count": 0}
        (self.ops / "xkrx-calendar-bootstrap.py").write_text(
            "import sys\nprint('2026-08-31')\nprint(" + repr(json.dumps(metadata)) + ",file=sys.stderr)\n")
        for name, args in (("kis-daily-production.sh", ["--check"]),
                           ("kis-daily-production.sh", ["--execute"]),
                           ("backfill-production.sh", ["--start", "2026-08-31", "--end", "2026-08-31", "--execute"])):
            with self.subTest(helper=name, mode=args[-1]):
                result = self.command(name, *args)
                self.assertEqual(result.returncode, 73)
                self.assertIn(b"PRODUCTION_CONFIG: PASS (scope=backfill", result.stdout)
                self.assertIn(b"SUPPLIER_REAL_VALIDATOR_REACHED", result.stdout)

    def test_on_supplier_keeps_mode_override_and_activation_guards(self):
        for key, value in (("OWNER_BETA_ACCESS_MODE", "disabled"),
                           ("OWNER_BETA_PRICE_INPUT_MODE", "disabled"),
                           ("OWNER_BETA_PAPER_MODE", "enabled"),
                           ("PRICE_BETA_DAILY_CONFIG_SHA256", "sha256:" + "b" * 64)):
            with self.subTest(key=key):
                old = self.values.get(key)
                self.values[key] = value
                self.write_env()
                self.assertEqual(self.command("validate-production-config.sh", "--scope", "backfill").returncode, 1)
                if old is None:
                    del self.values[key]
                else:
                    self.values[key] = old
        self.write_env()
        self.assertEqual(self.command("validate-production-config.sh", "--scope", "backfill",
            extra={"OWNER_BETA_DAILY_MODE": "disabled"}).returncode, 1)
        self.assertEqual(self.command("validate-production-config.sh", "--scope", "infrastructure").returncode, 1)


if __name__ == "__main__":
    unittest.main(verbosity=2)
