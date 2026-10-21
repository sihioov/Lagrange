#!/usr/bin/env python3
"""Root filesystem and real isolated PostgreSQL first-session transition tests.

Reuse the full-schema fixture's private database setup only. No backup run,
production database, installed release, real secret, or provider is accessed.
"""
from __future__ import annotations

import contextlib
import copy
import datetime as dt
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys
import types
import unittest
from unittest import mock

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    sys.modules[name] = value
    spec.loader.exec_module(value)
    return value


first = load("first_session_tested", ROOT / "scripts/ops/lib/owner-beta-daily-first-session.py")
fixture = load("first_session_database_fixture", ROOT / "scripts/ops/owner-beta-daily-backup-docker-self-test.py")
install = fixture.backup.install


class FirstSessionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Separate class prevents discovery/repetition of unrelated backup tests.
        cls.database = type("FirstSessionDatabase", (fixture.DailyBackupDockerTests,), {})
        cls.database.setUpClass()
        cls.addClassCleanup(cls.database.doClassCleanups)
        cls.source = types.SimpleNamespace(release=str(cls.database.release), commit=fixture.COMMIT)
        cls.old, _ = install.verify_config(cls.source.release, cls.source.commit)
        cls.today = dt.datetime.now(dt.timezone(dt.timedelta(hours=9))).date()
        cls.new = copy.deepcopy(cls.old)
        cls.new.update(code_commit="b" * 40, first_auto_session=(cls.today + dt.timedelta(days=1)).isoformat())
        cls.target = types.SimpleNamespace(release=str(cls.database.directory / "releases" / ("b" * 40)), commit="b" * 40)
        target = Path(cls.target.release)
        (target / "deploy/db").mkdir(parents=True)
        shutil.copyfile(ROOT / "deploy/db/owner-beta-daily-first-session.sql",
                        target / "deploy/db/owner-beta-daily-first-session.sql")
        source = cls.database.directory / "new-config.json"
        source.write_bytes(install.canonical(cls.new))
        source.chmod(0o600)
        install.stage_config(str(target), str(source), str(cls.database.directory / "daily-policy.json"), cls.target.commit)
        cls.values = {"MAX_JOBS_PER_OWNER": "4", "PRICE_BETA_LEASE_SECS": "60", "PRICE_BETA_BACKOFF_SECS": "1"}

    def sql(self, *, apply=True):
        return first.render(install, self.source, self.target, self.values, apply=apply)

    @staticmethod
    def body(sql):
        # Fixtures wrap the actual SQL body in one rolled-back transaction.
        return sql.replace("BEGIN;", "", 1).removesuffix("COMMIT;\n")

    def execute(self, sql, *, setup="", assertions="", role="migration_owner"):
        return self.database.command([
            "exec", "--interactive", "--user", "999:999", self.database.server,
            "psql", "-X", "-qAt", "-U", role, "-d", "lagrange",
            "-v", "ON_ERROR_STOP=1", "-v", "VERBOSITY=sqlstate"],
            ("BEGIN;\n" + setup + self.body(sql) + assertions + "\nROLLBACK;\n").encode(),
            allow_failure=True)

    def assert_rejected(self, result):
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(self.database.sqlstate(result), {"23514", "42501"})
        self.assertNotIn(b"DAILY_FIRST_SESSION:", result.stdout)

    def test_actual_preflight_does_not_change_control_or_epoch(self):
        result = self.execute(self.sql(apply=False), setup=
            "CREATE TEMP TABLE original AS SELECT to_jsonb(c) AS row FROM public.owner_beta_daily_control c;\n",
            assertions="DO $$ BEGIN IF (SELECT row FROM original) IS DISTINCT FROM "
            "(SELECT to_jsonb(c) FROM public.owner_beta_daily_control c) THEN RAISE EXCEPTION 'changed'; END IF; END $$;")
        self.assertEqual(result.returncode, 0, self.database.sqlstate(result))
        self.assertIn(b"DAILY_FIRST_SESSION: PREFLIGHT", result.stdout)

    def test_actual_apply_advances_epoch_once_preserves_other_fields_and_is_idempotent(self):
        body = self.body(self.sql())
        assertions = """
DO $$ BEGIN
 IF NOT EXISTS(SELECT 1 FROM public.owner_beta_daily_control c, original o
   WHERE NOT c.enabled AND c.activation_epoch=1
     AND c.first_auto_session=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date+1
     AND (to_jsonb(c)-ARRAY['first_auto_session','activation_epoch','updated_at'])=
         (o.row-ARRAY['first_auto_session','activation_epoch','updated_at'])) THEN
   RAISE EXCEPTION 'binding changed'; END IF;
END $$;
CREATE TEMP TABLE after_first AS SELECT to_jsonb(c) AS row FROM public.owner_beta_daily_control c;
DROP TABLE pg_temp.daily_first_session_input;
""" + body + """
DO $$ BEGIN IF (SELECT row FROM after_first) IS DISTINCT FROM
 (SELECT to_jsonb(c) FROM public.owner_beta_daily_control c) THEN
 RAISE EXCEPTION 'replay changed row'; END IF; END $$;
"""
        result = self.execute(self.sql(), setup=
            "CREATE TEMP TABLE original AS SELECT to_jsonb(c) AS row FROM public.owner_beta_daily_control c;\n",
            assertions=assertions)
        self.assertEqual(result.returncode, 0, self.database.sqlstate(result))
        self.assertEqual(result.stdout.count(b"REBOUND_DISABLED"), 2)

    def test_actual_enabled_and_previously_enabled_controls_are_rejected(self):
        for setup in (
            "UPDATE public.owner_beta_daily_control SET enabled=true;",
            "UPDATE public.owner_beta_daily_control SET enabled=true; UPDATE public.owner_beta_daily_control SET enabled=false;",
            "UPDATE public.owner_beta_daily_control SET lease_seconds=61;",
        ):
            with self.subTest(setup=setup):
                self.assert_rejected(self.execute(self.sql(), setup=setup))

    def test_actual_any_poll_history_is_rejected_without_deleting_it(self):
        for setup in (
            "INSERT INTO owner_beta_daily_poll_state(stage,next_poll_at) VALUES('SCHEDULE',now());",
            "INSERT INTO owner_beta_daily_poll_receipts(stage,owner_user_id,policy_sha256,activation_epoch) "
            "SELECT 'ADMISSION',owner_user_id,policy_sha256,activation_epoch FROM public.owner_beta_daily_control;",
        ):
            with self.subTest(setup=setup):
                self.assert_rejected(self.execute(self.sql(), setup=setup))

    def test_actual_sql_rejects_binding_and_date_tampering_independently_of_renderer(self):
        original = self.sql()
        for key, value in (
            ("owner", "00000000-0000-4000-8000-000000000002"),
            ("first_auto_session", self.today.isoformat()),
            ("first_auto_session", self.old["first_auto_session"]),
            ("code_commit", self.old["code_commit"]),
            ("code_commit", "0" * 40),
            ("entitlement_reference", "different"),
        ):
            new = {**self.new, key: value}
            sql = original.replace(install.canonical(self.new).hex(), install.canonical(new).hex())
            with self.subTest(key=key, value=value):
                self.assert_rejected(self.execute(sql))

    def test_actual_app_role_cannot_rebind(self):
        self.assert_rejected(self.execute(self.sql(), role="app"))

    def test_renderer_allows_only_date_and_revision_change(self):
        first.validate_pair(install, self.old, self.new, today=self.today)
        for key in set(self.new) - {"code_commit", "first_auto_session"}:
            bad = {**self.new, key: None}
            with self.subTest(key=key), self.assertRaises(install.DailyInstallError):
                first.validate_pair(install, self.old, bad, today=self.today)
        for bad in (self.old, {**self.new, "first_auto_session": self.today.isoformat()},
                    {**self.new, "code_commit": self.old["code_commit"]}):
            with self.assertRaises(install.DailyInstallError):
                first.validate_pair(install, self.old, bad, today=self.today)

    def test_pristine_filesystem_is_held_and_never_reset(self):
        root = self.database.directory / "pristine-state"
        root.mkdir(mode=0o755)
        self.addCleanup(shutil.rmtree, root)
        install.provision_state(str(root), self.new)
        state = root / "owner-beta-daily"
        before = {str(p): p.read_bytes() for p in state.rglob("*") if p.is_file()}
        with first.pristine_guard(install, str(root), self.new):
            with self.assertRaises(install.DailyInstallError):
                with first.pristine_guard(install, str(root), self.new):
                    pass
        self.assertEqual(before, {str(p): p.read_bytes() for p in state.rglob("*") if p.is_file()})
        candidate = state / "candidates" / "existing"
        candidate.write_bytes(b"retained")
        with self.assertRaises(install.DailyInstallError):
            with first.pristine_guard(install, str(root), self.new):
                pass
        self.assertEqual(candidate.read_bytes(), b"retained")
        candidate.unlink()  # Own disposable fixture only.
        checkpoint = state / "producer-state/checkpoint.json"
        value = json.loads(checkpoint.read_bytes())
        value.update(generation=1, last_started_unix_millis=1)
        checkpoint.write_bytes(install.canonical(value))
        with self.assertRaises(install.DailyInstallError):
            with first.pristine_guard(install, str(root), self.new):
                pass
        self.assertEqual(json.loads(checkpoint.read_bytes())["generation"], 1)

    @contextlib.contextmanager
    def operation_fixture(self):
        ops = fixture.backup.ops
        root = self.database.directory / "operation-state"
        root.mkdir(mode=0o755)
        try:
            install.provision_state(str(root), self.new)
            values = {**self.values, "LAGRANGE_DATA_DIR": str(root),
                      "LAGRANGE_ARTIFACTS_DIR": "/fixture-artifacts",
                      "LAGRANGE_RUNTIME_SECRET_DIR": "/fixture-secrets", "POSTGRES_DB": "lagrange"}
            source = types.SimpleNamespace(**vars(self.source))
            target = types.SimpleNamespace(**vars(self.target), values=values)
            database = mock.Mock()
            database.status.return_value = "DISABLED"
            database.execute.return_value = "DAILY_FIRST_SESSION: REBOUND_DISABLED"
            operation = types.SimpleNamespace(source=source, target=target, database=database,
                docker=object(), ownership=object(), install_root="/fixture-install")
            with mock.patch.object(ops, "stopped") as stopped, \
                    mock.patch.object(ops, "read_context", return_value=(dict(values), [])) as context, \
                    mock.patch.object(ops.install, "reader_guard"):
                yield ops, operation, stopped, context, root
        finally:
            shutil.rmtree(root)

    def test_operation_holds_poll_locks_for_database_and_rechecks_original_cohort(self):
        with self.operation_fixture() as (ops, operation, stopped, context, root):
            def execute(values, sql, expected):
                with self.assertRaises(install.DailyInstallError):
                    with first.pristine_guard(install, str(root), self.new):
                        pass
                self.assertEqual(expected, {"DAILY_FIRST_SESSION: REBOUND_DISABLED"})
                return "DAILY_FIRST_SESSION: REBOUND_DISABLED"
            operation.database.execute.side_effect = execute
            self.assertEqual(first.run(ops, operation, {}, apply=True), "REBOUND_DISABLED")
            self.assertEqual(stopped.call_count, 3)
            operation.database.execute.assert_called_once()

    def test_operation_failures_never_reach_database_transition(self):
        for failure in ("running", "context", "enabled", "history"):
            with self.subTest(failure=failure), self.operation_fixture() as (ops, operation, stopped, context, root):
                if failure == "running":
                    stopped.side_effect = ops.runtime.RuntimeErrorCode("DAILY_OPERATION_NOT_QUIESCENT")
                elif failure == "context":
                    context.return_value[0]["POSTGRES_DB"] = "different"
                elif failure == "enabled":
                    operation.database.status.return_value = "ENABLED"
                else:
                    (root / "owner-beta-daily/candidates/retained").write_bytes(b"history")
                with self.assertRaises((ops.runtime.RuntimeErrorCode, install.DailyInstallError)):
                    first.run(ops, operation, {}, apply=True)
                operation.database.execute.assert_not_called()


if __name__ == "__main__":
    unittest.main(verbosity=2)
