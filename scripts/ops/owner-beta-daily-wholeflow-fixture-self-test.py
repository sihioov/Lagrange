#!/usr/bin/env python3
"""Synthetic fixture/driver boundary tests; never approved-base execution.

Hosted CI runs this file as root, without private base data, Docker or credentials.
The Git ownership cases use real Git only in newly created disposable repositories.
Mocked driver tests establish ordering/failure behavior only. They do not prove
Rust compilation, SQL admission, real HTTP publication or actual recovery.
"""
from __future__ import annotations

import copy
import datetime as dt
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("wholeflow_driver_tested", ROOT / "scripts/ops/verify-owner-beta-daily-wholeflow.py")
driver = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = driver
spec.loader.exec_module(driver)
f = driver.fixture
COMMIT = "a" * 40
OWNER = "00000000-0000-4000-8000-000000000001"
NOW = dt.datetime(2026, 10, 16, 10, 0, tzinfo=dt.timezone.utc)


class WholeflowFixtureTests(unittest.TestCase):
    def setUp(self):
        # Any accidental subprocess/provider use in the fixture suite fails.
        self.addCleanup(mock.patch.stopall)
        mock.patch.object(subprocess, "run", side_effect=AssertionError("UNEXPECTED_EXECUTION")).start()
        self.plan = f.plan(NOW, OWNER, COMMIT)

    def batches(self):
        return [json.loads(value) for key, value in self.plan["raw_files"].items() if key.endswith("/batch.json")]

    def test_two_sessions_and_all_provider_bytes_exist_before_execution(self):
        self.assertEqual(self.plan["sessions"], ["2026-08-31", "2026-10-16"])
        self.assertIs(self.plan["synthetic"], True)
        batches = self.batches()
        self.assertEqual(sorted(len(value["files"]) for value in batches), [1, 29, 30])
        self.assertEqual(sum(len(b["files"]) for b in batches), 60)
        self.assertEqual(set(b["provider"] for b in batches), {"kis", "kis-calendar"})

    def test_every_committed_body_and_metadata_match_both_manifest_scopes(self):
        values = self.plan["raw_files"]
        committed = set()
        for provider in ("kis", "kis-calendar"):
            prefix = f"manifests/provider={provider}/market=kr/"
            self.assertEqual(values[prefix + "commit.lock"], b"")
            for line in values[prefix + "manifest.jsonl"].splitlines():
                batch = json.loads(line)
                root = f"provider={provider}/market=kr/date={batch['date']}/batch={batch['batch_id']}"
                self.assertEqual(values[root + "/batch.json"], line)
                for entry in batch["files"]:
                    path = root + "/" + entry["file_name"]
                    self.assertEqual(entry["size_bytes"], len(values[path]))
                    self.assertEqual(entry["content_hash"], f.digest(values[path]))
                    self.assertIsNone(entry["copied_from"])
                    committed.add(path)
        actual = {path for path in values if path.startswith("provider=") and not path.endswith("/batch.json")}
        self.assertEqual(committed, actual)

    def test_original_policy_bytes_and_real_installed_paths_are_retained(self):
        policy = (ROOT / "configs/evidence/owner-beta-daily-price-policy-v1.json").read_bytes()
        driver.release.validate_policy(policy)
        config = driver.release.validate_config(f.canonical(self.plan["config"]), COMMIT)
        self.assertEqual(config["policy_sha256"], f.digest(policy))
        self.assertEqual(config["raw_root"], "/data/raw")
        self.assertEqual(config["historical_root"], "/data/owner-beta-artifacts")
        self.assertNotIn("approval_registry", config)
        self.assertNotIn("clock", config)

    def test_synthetic_calendar_covers_every_civil_date_and_only_two_trading_days(self):
        calendars = [json.loads(v)["output"] for k, v in self.plan["raw_files"].items() if k.endswith("/calendar.json")]
        self.assertEqual(len(calendars), 2)
        self.assertEqual(calendars[0], calendars[1])
        rows = calendars[0]
        self.assertEqual([row["bass_dt"] for row in rows if row["opnd_yn"] == "Y"], ["20260831", "20261016"])
        dates = [dt.datetime.strptime(row["bass_dt"], "%Y%m%d").date() for row in rows]
        self.assertEqual(dates, [f.START + dt.timedelta(days=i) for i in range(len(dates))])
        self.assertEqual(dates[-1].isoformat(), self.plan["sessions"][-1])

    def test_calendar_cutoff_uses_real_seoul_1630_boundary(self):
        before = dt.datetime(2026, 10, 16, 7, 29, tzinfo=dt.timezone.utc)
        at = before + dt.timedelta(minutes=1)
        self.assertEqual(f.eligible(before), dt.date(2026, 10, 15))
        self.assertEqual(f.eligible(at), dt.date(2026, 10, 16))

    def test_calendar_cutoff_and_range_fail_before_any_files_are_written(self):
        for now in (dt.datetime(2026, 8, 31, 12, tzinfo=dt.timezone.utc),
                    dt.datetime(2027, 10, 1, 12, tzinfo=dt.timezone.utc),
                    dt.datetime(2026, 10, 16, 7, 29, tzinfo=dt.timezone.utc)):
            with self.subTest(now=now), self.assertRaises(f.FixtureError):
                f.plan(now, OWNER, COMMIT)
        with self.assertRaises(f.FixtureError):
            f.plan(NOW.replace(tzinfo=None), OWNER, COMMIT)

    def test_action_queries_prove_seven_channels_for_each_exact_addition_range(self):
        batches = sorted((b for b in self.batches() if b["provider"] == "kis"), key=lambda b: b["date"])
        for batch, interval in zip(batches, [("20260829", "20260831"), ("20260901", "20261016")], strict=True):
            actions = [e for e in batch["files"] if e["kind"] == "corporate_actions"]
            self.assertEqual(len(actions), 7)
            identities = set()
            for entry in actions:
                query = dict(entry["request"]["query"])
                self.assertEqual((query["F_DT"], query["T_DT"]), interval)
                self.assertEqual((query["SHT_CD"], query["CTS"]), ("", ""))
                self.assertEqual(entry["response_continuation"], "F")
                identities.add((entry["request"]["endpoint"], query.get("GB1")))
            self.assertEqual(len(identities), 7)

    def test_price_reference_universe_is_complete_and_sessions_have_distinct_prices(self):
        for batch in self.batches():
            if batch["provider"] != "kis":
                continue
            for kind in ("bars", "reference"):
                entries = [e for e in batch["files"] if e["kind"] == kind]
                self.assertEqual({dict(e["request"]["query"])["FID_INPUT_ISCD"] for e in entries}, set(f.SYMBOLS))
            self.assertEqual(batch["entitlement_reference"], f.ENTITLEMENT)
        values = [json.loads(v)["output2"][0]["stck_clpr"] for k, v in self.plan["raw_files"].items()
                  if k.endswith("/bar-069500.json")]
        self.assertEqual(set(values), {"110", "111"})

    def test_raw_credentials_are_redacted_and_no_live_host_is_constructed(self):
        for batch in self.batches():
            for entry in batch["files"]:
                request = entry["request"]
                self.assertTrue(request["endpoint"].startswith("/uapi/domestic-stock/v1/"))
                headers = dict(request["headers"])
                for key in ("appkey", "appsecret", "authorization"):
                    self.assertEqual(headers[key], "[REDACTED]")
                self.assertEqual(headers["tr_cont"], "")
                self.assertEqual(request["mode"], "credentialed")

    def test_repeated_plan_has_identical_bytes_without_retrieval_time_rewrite(self):
        self.assertEqual(self.plan, f.plan(NOW, OWNER, COMMIT))
        self.assertTrue(all(b["retrieved_at"] == "2026-10-16T10:00:00Z" for b in self.batches()))
        different = f.plan(NOW, "00000000-0000-4000-8000-000000000002", COMMIT)
        self.assertNotEqual(self.plan["raw_files"], different["raw_files"])

    @staticmethod
    def manifest():
        return ("LAGRANGE_RELEASE_MANIFEST_V2\ncommit|" + COMMIT + "\n" + "".join(
            f"image|{service}|lagrange-station-{service}:{COMMIT}|sha256:{index:064x}|{COMMIT}\n"
            for index, service in enumerate(f.SERVICES, 1))).encode()

    def test_manifest_requires_all_twelve_exact_ordered_candidates(self):
        data = self.manifest()
        self.assertEqual(tuple(f.image_manifest(data, COMMIT)), f.SERVICES)
        lines = data.splitlines(keepends=True)
        mutations = [data.rstrip(), b"".join(lines[:-1]), data + lines[-1],
                     b"".join(lines[:2] + [lines[3], lines[2]] + lines[4:]),
                     data.replace(b"lagrange-station-web:" + COMMIT.encode(), b"lagrange-station-web:latest"),
                     data.replace(b"\n", b"\r\n"), data.replace(b"image|web|", b"image|api-server|")]
        for mutated in mutations:
            with self.subTest(data=mutated[:40]), self.assertRaises(f.FixtureError):
                f.image_manifest(mutated, COMMIT)

    @staticmethod
    def admission_rows():
        return [{"range_start": "2016-08-29", "range_end": date, "session_count": 2453 + n,
                 "bar_count": 11 * (2453 + n), "admission_sha256": "sha256:" + str(n + 1) * 64,
                 "content_sha256": "sha256:" + str(n + 3) * 64, "manifest_sha256": "sha256:" + str(n + 5) * 64,
                 "parent_admission_sha256": None if n == 0 else "sha256:" + "1" * 64}
                for n, date in enumerate(("2026-08-31", "2026-10-16"))]

    def test_admission_assertion_rejects_missing_parent_replaced_base_and_wrong_counts(self):
        rows = self.admission_rows()
        f.audit_admissions(rows, self.plan["sessions"])
        for key, value in (("parent_admission_sha256", None), ("bar_count", 11),
                           ("range_start", "2026-08-31"), ("range_end", "2026-10-15"),
                           ("admission_sha256", rows[0]["admission_sha256"])):
            changed = copy.deepcopy(rows)
            changed[1][key] = value
            with self.subTest(key=key), self.assertRaises(f.FixtureError):
                f.audit_admissions(changed, self.plan["sessions"])

    def test_healthy_or_exit_zero_does_not_substitute_for_substantive_outcome(self):
        driver.fixed_outcome(b'{"event":"owner_beta_daily_cycle","outcome":"ADMITTED"}',
                             "owner_beta_daily_cycle", "ADMITTED")
        for data in (b"", b"healthy", b'{"event":"owner_beta_daily_cycle","outcome":"NOT_DUE"}',
                     b'{"event":"owner_beta_daily_cycle","code":"DATABASE"}',
                     b'{"event":"owner_beta_daily_cycle","outcome":"ADMITTED"}\n' * 2):
            with self.subTest(data=data), self.assertRaises(f.FixtureError):
                driver.fixed_outcome(data, "owner_beta_daily_cycle", "ADMITTED")

    def test_http_publication_requires_bound_current_run_and_exact_asof(self):
        state = {"enabled": True, "runtime_enabled": True, "state": "CURRENT", "reason_code": None,
                 "expected_session": "2026-10-16", "data_support_end": "2026-10-16",
                 "recommendation_as_of": "2026-10-16", "input_kind": "owner_beta_daily_etf11_price_only_v1",
                 "trigger_kind": "DAILY", "published_at": "2026-10-16T10:01:00Z", "current_reservation_run_id": OWNER}
        admission = self.admission_rows()[-1]
        run = {"id": OWNER, "status": "SUCCEEDED", "as_of": "2026-10-16", "trigger_kind": "DAILY",
               "admission_sha256": admission["admission_sha256"], "factor_snapshot_sha256": "sha256:" + "a" * 64,
               "target_snapshot_sha256": "sha256:" + "b" * 64}
        f.audit_publication(state, run, admission, "2026-10-16")
        for key, value in (("state", "UNVERIFIABLE"), ("reason_code", "VERIFICATION_EXPIRED"),
                           ("recommendation_as_of", "2026-10-15"), ("current_reservation_run_id", "other")):
            changed = dict(state, **{key: value})
            with self.subTest(key=key), self.assertRaises(f.FixtureError):
                f.audit_publication(changed, run, admission, "2026-10-16")

    def test_driver_gives_producer_no_database_network_or_secret(self):
        test = driver.WholeFlow(types.SimpleNamespace(timeout_seconds=1200, candidate=COMMIT))
        test.config_root, test.base, test.raw, test.daily = map(Path, ("/fixture/config", "/fixture/base", "/fixture/raw", "/fixture/daily"))
        test.config_bytes, test.network, test.images = b"{}", "private-network", {"owner-beta-runner": "sha256:" + "a" * 64}
        output = b'{"event":"owner_beta_daily_cycle","outcome":"CANDIDATE_READY"}'
        with mock.patch.object(test, "once", return_value=output) as once, mock.patch.object(test, "db_env") as db:
            test.daily_once("producer", "CANDIDATE_READY")
        args = once.call_args.args
        self.assertEqual((args[2], args[3]), (10002, "none"))
        self.assertFalse(any("SECRET" in key or key.startswith("DB") for key in args[5]))
        db.assert_not_called()
        self.assertIn("type=bind,src=/fixture/base,dst=/data/owner-beta-artifacts,readonly", args[4])
        self.assertIn("type=bind,src=/fixture/raw,dst=/data/raw,readonly", args[4])

    def test_replay_waits_actual_poll_and_rejects_a_replaced_run(self):
        test = driver.WholeFlow(types.SimpleNamespace(timeout_seconds=1200, candidate=COMMIT))
        events = []
        with mock.patch.object(test, "invariant", side_effect=[{"run": 1}, {"run": 2}]), \
             mock.patch.object(test, "pause_poll", side_effect=lambda: events.append("wait")), \
             mock.patch.object(test, "daily_once", side_effect=lambda *args: events.append(args)), \
             mock.patch.object(test, "publication") as published:
            with self.assertRaisesRegex(f.FixtureError, "WHOLEFLOW_REPLAY_IDENTITY"):
                test.replay("ignored")
        self.assertEqual(events, ["wait", ("producer", "REPLAY"), ("admitter", "REPLAY"), ("worker", "IDLE")])
        published.assert_not_called()

    def test_lost_create_reply_without_observed_object_retains_private_stage(self):
        test = driver.WholeFlow(types.SimpleNamespace(timeout_seconds=1200, candidate=COMMIT))
        with tempfile.TemporaryDirectory() as temp:
            test.root = Path(temp)
            test.uncertain_creations.add(("container", "pending-name"))
            with mock.patch.object(test, "command", return_value=b""):
                with self.assertRaisesRegex(f.FixtureError, "WHOLEFLOW_CREATE_REPLY_UNRESOLVED"):
                    test.cleanup()
            self.assertTrue(test.root.is_dir())
            self.assertNotIn("cleanup", test.receipt)

    def test_foreign_cleanup_identity_is_never_removed(self):
        test = driver.WholeFlow(types.SimpleNamespace(timeout_seconds=1200, candidate=COMMIT))
        with tempfile.TemporaryDirectory() as temp:
            test.root = Path(temp)
            with mock.patch.object(test, "command", return_value=b"unknown\n") as command, \
                 mock.patch.object(test, "inspect", return_value={"Config": {"Labels": {driver.LABEL: "foreign"}},
                                   "Id": "unknown", "Name": "/unowned"}):
                with self.assertRaisesRegex(f.FixtureError, "WHOLEFLOW_CLEANUP_FOREIGN"):
                    test.cleanup()
            self.assertEqual(command.call_count, 1)
            self.assertTrue(test.root.is_dir())

    def test_creation_intent_is_written_before_daemon_request(self):
        test = driver.WholeFlow(types.SimpleNamespace(timeout_seconds=1200, candidate=COMMIT))
        events = []
        with mock.patch.object(test, "intent", side_effect=lambda *args: events.append("intent")), \
             mock.patch.object(test, "command", side_effect=lambda *args: events.append("create") or
                               (_ for _ in ()).throw(f.FixtureError("WHOLEFLOW_DOCKER_UNCERTAIN"))):
            with self.assertRaises(f.FixtureError):
                test.create_network("initial")
        self.assertEqual(events, ["intent", "create"])
        self.assertEqual(test.uncertain_creations, {("network", test.nonce + "-initial")})


class WholeflowGitSourceTests(unittest.TestCase):
    def setUp(self):
        self.assertEqual(os.geteuid(), 0, "Git ownership fixtures require root; do not skip them")
        temporary = tempfile.TemporaryDirectory(prefix="wholeflow-git-fixture-")
        self.addCleanup(temporary.cleanup)
        self.repo = Path(temporary.name) / "operator-checkout"
        self.repo.mkdir()
        self.env = dict(driver.ENV, GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_SYSTEM="/dev/null")
        # Independent fixtures for every protected source path. No real
        # approved base, production repository or database is opened here.
        self.paths = (
            "configs/evidence/owner-beta-daily-price-policy-v1.json",
            "deploy/db/owner-beta-daily-control.sql",
            "scripts/ops/verify-owner-beta-daily-wholeflow.py",
            "scripts/ops/lib/owner-beta-daily-wholeflow-fixture.py",
            "scripts/ops/lib/owner-beta-daily-release.py",
            "scripts/ops/lib/owner-beta-daily-backup-files.py",
        )
        self.git("init", "--quiet")
        for relative in self.paths:
            path = self.repo / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((relative + "\n").encode())
        self.git("add", "--all")
        self.git("-c", "user.name=Wholeflow fixture", "-c", "user.email=fixture@example.invalid",
                 "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null",
                 "commit", "--quiet", "-m", "source fixture")
        self.commit = self.git("rev-parse", "HEAD").stdout.decode().strip()
        self.config_before = (self.repo / ".git/config").read_bytes()
        for path in [self.repo, *self.repo.rglob("*")]:
            os.chown(path, 10001, 10001)
        root_patch = mock.patch.object(driver, "ROOT", self.repo)
        env_patch = mock.patch.object(driver, "ENV", self.env)
        root_patch.start()
        env_patch.start()
        self.addCleanup(root_patch.stop)
        self.addCleanup(env_patch.stop)

    def git(self, *args, check=True):
        return subprocess.run(["git", "-C", str(self.repo), *args], env=self.env,
                              capture_output=True, timeout=10, check=check)

    def test_root_reads_operator_checkout_without_persistent_git_trust(self):
        before = self.git("rev-parse", "HEAD", check=False)
        self.assertNotEqual(before.returncode, 0)
        self.assertIn(b"detected dubious ownership", before.stderr)
        driver.verify_source(self.commit)
        after = self.git("rev-parse", "HEAD", check=False)
        self.assertNotEqual(after.returncode, 0)
        self.assertIn(b"detected dubious ownership", after.stderr)
        self.assertEqual((self.repo / ".git/config").read_bytes(), self.config_before)
        self.assertEqual(self.repo.stat().st_uid, 10001)

    def test_root_source_check_rejects_different_candidate(self):
        wrong = "0" * 40 if self.commit != "0" * 40 else "1" * 40
        with self.assertRaisesRegex(f.FixtureError, "WHOLEFLOW_SOURCE_REVISION"):
            driver.verify_source(wrong)

    def test_root_source_check_rejects_each_modified_protected_file(self):
        for relative in self.paths:
            path = self.repo / relative
            original = path.read_bytes()
            try:
                path.write_bytes(original + b"uncommitted change\n")
                with self.subTest(path=relative), self.assertRaisesRegex(f.FixtureError, "WHOLEFLOW_SOURCE_CHANGED"):
                    driver.verify_source(self.commit)
            finally:
                path.write_bytes(original)

    def test_root_git_failure_has_only_a_finite_diagnostic(self):
        (self.repo / ".git/HEAD").unlink()
        with self.assertRaises(f.FixtureError) as raised:
            driver.verify_source(self.commit)
        self.assertEqual(str(raised.exception), "WHOLEFLOW_SOURCE_GIT_FAILED")
        self.assertNotIn(str(self.repo), str(raised.exception))


if __name__ == "__main__":
    unittest.main(verbosity=2)
