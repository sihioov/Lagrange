#!/usr/bin/env python3
"""Actual disposable Docker/psql transport, using only synthetic local credentials."""
from __future__ import annotations
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
import uuid
from unittest import mock

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "daily_transport_tested", ROOT / "scripts/ops/lib/owner-beta-daily-operations.py")
ops = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = ops
spec.loader.exec_module(ops)
POSTGRES = "postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a"
COMMIT = "a" * 40
TABLES = ('owner_beta_daily_control', 'owner_beta_daily_admissions', 'owner_beta_daily_admission_events', 'owner_beta_daily_calendar_sources', 'owner_beta_daily_calendar_days', 'owner_beta_daily_source_events', 'owner_beta_daily_poll_receipts', 'owner_beta_daily_poll_state', 'owner_beta_daily_reservations', 'owner_beta_daily_publication_claims')


class RecordingDocker(ops.runtime.Docker):
    def __init__(self):
        super().__init__()
        self.observations = []

    def object(self, kind, identity, timeout=15):
        value = super().object(kind, identity, timeout)
        if kind == "container" and (value.get("Config", {}).get("Labels") or {}).get(
                "io.lagrange.daily-control"):
            # Memory only. Tests never dump this structure or container env.
            self.observations.append(value)
        return value


class DailyTransportTests(unittest.TestCase):
    @classmethod
    def command(cls, args, payload=None, *, timeout=90, allow_failure=False):
        result = subprocess.run(
            ["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", *args],
            env=ops.ENVIRONMENT, input=payload, capture_output=True, timeout=timeout, check=False)
        if not allow_failure and result.returncode != 0:
            raise RuntimeError("DAILY_TRANSPORT_FIXTURE_COMMAND")
        return result

    @classmethod
    def setUpClass(cls):
        if os.geteuid() != 0 or sys.platform != "linux":
            raise RuntimeError("DAILY_TRANSPORT_ACTUAL_ROOT_REQUIRED")
        cls.directory = Path(tempfile.mkdtemp(prefix="lagrange-daily-transport-", dir="/var/lib"))
        cls.project = "daily-transport-" + uuid.uuid4().hex[:16]
        cls.network = cls.server = cls.image = None
        cls.addClassCleanup(cls.cleanup)
        # Exercise the same protected-root read as an installed release. The
        # checkout itself is deliberately not treated as a root-owned release.
        release = cls.directory / "releases" / COMMIT
        (release / "deploy/db").mkdir(parents=True)
        shutil.copyfile(ROOT / "deploy/db/owner-beta-daily-control-status.sql",
                        release / "deploy/db/owner-beta-daily-control-status.sql")
        root_patch = mock.patch.object(ops, "ROOT", str(release))
        root_patch.start()
        cls.addClassCleanup(root_patch.stop)
        cls.secret_root = cls.directory / "secrets"
        cls.secret_root.mkdir(mode=0o750)
        service = cls.secret_root / "db-migrate"
        service.mkdir(mode=0o750)
        os.chown(service, 999, 999)
        cls.secret = service / "db_migration_owner_password"
        cls.password = uuid.uuid4().hex + uuid.uuid4().hex
        cls.secret.write_text(cls.password)
        os.chown(cls.secret, 999, 999)
        cls.secret.chmod(0o400)
        build = cls.directory / "build"
        build.mkdir()
        shutil.copyfile(ROOT / "deploy/db/daily-control.sh", build / "daily-control.sh")
        (build / "Dockerfile").write_text(
            "FROM " + POSTGRES + "\n"
            "COPY daily-control.sh /usr/local/bin/lagrange-daily-control\n"
            "RUN chmod 0755 /usr/local/bin/lagrange-daily-control\n"
            'LABEL org.opencontainers.image.revision="' + COMMIT + '"\n'
            "USER 999:999\n"
            'ENTRYPOINT ["/usr/local/bin/lagrange-daily-control"]\n')
        iid = cls.directory / "image-id"
        cls.command(["build", "--pull=false", "--iidfile", str(iid), str(build)], timeout=180)
        cls.image = iid.read_text().strip()
        if not ops.runtime.DIGEST.fullmatch(cls.image):
            raise RuntimeError("DAILY_TRANSPORT_FIXTURE_IMAGE")
        cls.network = cls.command([
            "network", "create", "--internal", "--label",
            "com.docker.compose.project=" + cls.project, "--label",
            "com.docker.compose.network=owner-beta-db", cls.project + "_owner-beta-db"
        ]).stdout.decode().strip()
        cls.server = cls.command([
            "run", "--detach", "--name", cls.project + "-postgres", "--network", cls.network,
            "--network-alias", "postgres", "--label", "com.docker.compose.project=" + cls.project,
            "--label", "com.docker.compose.service=postgres",
            "--env", "POSTGRES_DB=lagrange", "--env",
            "POSTGRES_PASSWORD_FILE=/run/secrets/db_migration_owner_password",
            "--mount", "type=bind,src=" + str(cls.secret) +
            ",dst=/run/secrets/db_migration_owner_password,readonly", POSTGRES
        ]).stdout.decode().strip()
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            ready = cls.command(["exec", cls.server, "pg_isready", "-U", "postgres", "-d", "lagrange"],
                                allow_failure=True)
            if ready.returncode == 0:
                break
            time.sleep(0.25)
        else:
            raise RuntimeError("DAILY_TRANSPORT_FIXTURE_DATABASE")
        # Input travels over stdin; no password appears in Docker argv or output.
        cls.sql("CREATE ROLE migration_owner LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE "
                "PASSWORD '" + cls.password + "';\n"
                "GRANT CONNECT ON DATABASE lagrange TO migration_owner;\n"
                "GRANT USAGE,CREATE ON SCHEMA public TO migration_owner;\n")

    @classmethod
    def sql(cls, text):
        cls.command(["exec", "--interactive", cls.server, "psql", "-X", "-q", "-U", "postgres",
                     "-d", "lagrange", "-v", "ON_ERROR_STOP=1"], text.encode(), timeout=30)

    @classmethod
    def cleanup(cls):
        failures = []
        def remove(args):
            try:
                if cls.command(args, allow_failure=True).returncode != 0:
                    failures.append("cleanup")
            except Exception:
                failures.append("cleanup")
        if cls.server:
            # Forced deletion is restricted to this owned disposable fixture.
            remove(["rm", "--force", "--volumes", cls.server])
        try:
            leftover = cls.command(["ps", "--all", "--quiet", "--no-trunc", "--filter",
                                    "label=io.lagrange.daily-control-project=" + cls.project])
            for identity in leftover.stdout.decode().splitlines():
                if not ops.runtime.CONTAINER.fullmatch(identity):
                    failures.append("identity")
                else:
                    remove(["rm", "--force", "--volumes", identity])
        except Exception:
            failures.append("enumeration")
        if cls.network:
            remove(["network", "rm", cls.network])
        if cls.image:
            remove(["image", "rm", cls.image])
        shutil.rmtree(cls.directory)
        if failures:
            raise RuntimeError("DAILY_TRANSPORT_FIXTURE_CLEANUP")

    def setUp(self):
        self.sql("DROP TABLE IF EXISTS public.owner_beta_recommendation_runs;\n"
                 "DROP TABLE IF EXISTS public.users;\n" +
                 "".join("DROP TABLE IF EXISTS public." + name + ";\n" for name in TABLES))
        self.secret.write_text(self.password)
        self.secret.chmod(0o400)
        self.docker = RecordingDocker()
        self.database = ops.Database(self.docker, self.project, COMMIT, self.image)
        self.values = {"POSTGRES_DB": "lagrange", "LAGRANGE_RUNTIME_SECRET_DIR": str(self.secret_root)}

    def assert_no_oneshot(self):
        result = self.command(["ps", "--all", "--quiet", "--no-trunc", "--filter",
                               "label=io.lagrange.daily-control-project=" + self.project])
        self.assertEqual(result.stdout, b"")

    def test_actual_transport_distinguishes_four_bootstrap_states(self):
        self.assertEqual(self.database.status(self.values), "ABSENT_SCHEMA")
        self.sql("CREATE TABLE public.owner_beta_daily_control(id text,enabled boolean);\n"
                 "ALTER TABLE public.owner_beta_daily_control OWNER TO migration_owner;\n" +
                 "".join("CREATE TABLE public." + name + "(evidence integer); "
                         "ALTER TABLE public." + name + " OWNER TO migration_owner;\n"
                         for name in TABLES[1:]))
        self.assertEqual(self.database.status(self.values), "ABSENT_CONTROL")
        self.sql("INSERT INTO public.owner_beta_daily_control VALUES('etf11',false);\n")
        self.assertEqual(self.database.status(self.values), "DISABLED")
        self.sql("UPDATE public.owner_beta_daily_control SET enabled=true;\n")
        self.assertEqual(self.database.status(self.values), "ENABLED")
        self.assert_no_oneshot()

    def test_actual_partial_schema_and_orphaned_history_are_not_bootstrap(self):
        self.sql("CREATE TABLE public.owner_beta_daily_admissions(evidence integer); "
                 "ALTER TABLE public.owner_beta_daily_admissions OWNER TO migration_owner;\n")
        with self.assertRaises(ops.runtime.RuntimeErrorCode):
            self.database.status(self.values)
        self.sql("CREATE TABLE public.owner_beta_daily_control(id text,enabled boolean); "
                 "ALTER TABLE public.owner_beta_daily_control OWNER TO migration_owner;\n" +
                 "".join("CREATE TABLE public." + name + "(evidence integer); "
                         "ALTER TABLE public." + name + " OWNER TO migration_owner;\n"
                         for name in TABLES[2:]) +
                 "INSERT INTO public.owner_beta_daily_admissions VALUES(1);\n")
        with self.assertRaises(ops.runtime.RuntimeErrorCode):
            self.database.status(self.values)
        self.assert_no_oneshot()

    def test_actual_actor_rls_surviving_run_is_not_empty_history(self):
        self.sql("CREATE TABLE public.users(id uuid PRIMARY KEY); "
                 "ALTER TABLE public.users OWNER TO migration_owner; "
                 "CREATE TABLE public.owner_beta_recommendation_runs(owner_user_id uuid,input_kind text); "
                 "ALTER TABLE public.owner_beta_recommendation_runs OWNER TO migration_owner; "
                 "ALTER TABLE public.owner_beta_recommendation_runs ENABLE ROW LEVEL SECURITY; "
                 "ALTER TABLE public.owner_beta_recommendation_runs FORCE ROW LEVEL SECURITY; "
                 "CREATE POLICY actor_read ON public.owner_beta_recommendation_runs FOR SELECT "
                 "TO migration_owner USING(owner_user_id=NULLIF(current_setting('app.actor_user_id',true),'')::uuid); "
                 "INSERT INTO public.users VALUES('11111111-1111-4111-8111-111111111111'); "
                 "INSERT INTO public.owner_beta_recommendation_runs VALUES("
                 "'11111111-1111-4111-8111-111111111111','owner_beta_historical_price_only_v1');\n")
        self.assertEqual(self.database.status(self.values), "ABSENT_SCHEMA")
        self.sql("UPDATE public.owner_beta_recommendation_runs "
                 "SET input_kind='owner_beta_daily_etf11_price_only_v1';\n")
        with self.assertRaises(ops.runtime.RuntimeErrorCode):
            self.database.status(self.values)
        self.assert_no_oneshot()

    def test_actual_wrong_password_is_failure_and_container_is_removed(self):
        self.secret.write_text("different-synthetic-password")
        with self.assertRaises(ops.runtime.RuntimeErrorCode):
            self.database.status(self.values)
        self.assert_no_oneshot()

    def test_actual_failed_sql_is_not_disabled_and_container_is_removed(self):
        with self.assertRaises(ops.runtime.RuntimeErrorCode):
            self.database.execute(self.values, "SELECT 1/0;\n", {"DAILY_CONTROL: DISABLED"})
        self.assert_no_oneshot()

    def test_actual_control_container_has_only_internal_db_and_migration_secret(self):
        self.assertEqual(self.database.status(self.values), "ABSENT_SCHEMA")
        values = [v for v in self.docker.observations if v["State"].get("Status") == "exited"]
        self.assertTrue(values)
        for value in values:
            self.assertEqual(value["Config"]["User"], "999:999")
            self.assertTrue(value["HostConfig"]["ReadonlyRootfs"])
            self.assertFalse(value["HostConfig"]["Privileged"])
            self.assertEqual(value["HostConfig"]["CapDrop"], ["ALL"])
            self.assertEqual(value["HostConfig"]["RestartPolicy"]["Name"], "no")
            binds = [m for m in value["Mounts"] if m["Type"] == "bind"]
            self.assertEqual(len(binds), 1)
            self.assertEqual(binds[0]["Destination"], "/run/secrets/db_migration_owner_password")
            self.assertFalse(binds[0]["RW"])
            self.assertFalse(any(m["Type"] == "volume" for m in value["Mounts"]))
            self.assertEqual(len(value["NetworkSettings"]["Networks"]), 1)
            self.assertTrue(all(v["NetworkID"] == self.network
                                for v in value["NetworkSettings"]["Networks"].values()))
            env = ops.runtime.environment(value)
            self.assertNotIn("PGPASSWORD", env)
            self.assertNotIn("DATABASE_URL", env)
        self.assert_no_oneshot()

    def test_actual_entrypoint_refuses_nonmigration_user(self):
        result = self.command([
            "run", "--rm", "--interactive", "--network", "none", "--read-only",
            "--env", "DB_USER=worker", self.image], b"SELECT 1;\n", allow_failure=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b"")
        self.assertEqual(result.stderr, b"DAILY_CONTROL_DATABASE: FAILED\n")

    def test_actual_entrypoint_refuses_alternate_database_url(self):
        result = self.command([
            "run", "--rm", "--interactive", "--network", "none", "--read-only",
            "--env", "DB_USER=migration_owner", "--env", "DB_HOST=postgres", "--env", "DB_PORT=5432",
            "--env", "DB_NAME=lagrange", "--env",
            "DB_PASSWORD_FILE=/run/secrets/db_migration_owner_password",
            "--env", "DATABASE_URL=postgresql://synthetic-invalid", self.image],
            b"SELECT 1;\n", allow_failure=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b"")
        self.assertEqual(result.stderr, b"DAILY_CONTROL_DATABASE: FAILED\n")

    def test_actual_backup_audit_transport_accepts_bounded_json_beyond_control_limit(self):
        # Fixed SQL semantics are covered against all migrations by the Rust
        # backup tests. Here the real psql/container path exercises response
        # size, exact login, JSON decoding and cleanup, using synthetic output.
        sql = "SELECT jsonb_build_object('schema','lagrange-daily-backup-db-v1'," \
              "'login',current_user,'session',session_user,'fixture',repeat('x',5000))::text;"
        with mock.patch.object(ops.install, "read_root_file", return_value=sql.encode("ascii")):
            result = self.database.backup_audit(self.values)
        self.assertEqual(result["login"], "migration_owner")
        self.assertEqual(result["session"], "migration_owner")
        self.assertEqual(len(result["fixture"]), 5000)
        # A large arbitrary response still cannot use the ordinary control API.
        with self.assertRaises(ops.runtime.RuntimeErrorCode):
            self.database.execute(self.values, sql, {"DAILY_CONTROL: DISABLED"})
        self.assert_no_oneshot()

    def test_actual_backup_audit_rejects_unpinned_sql_duplicate_json_and_extra_rows(self):
        fixed = "SELECT '{\"schema\":\"lagrange-daily-backup-db-v1\"}';"
        with mock.patch.object(ops.install, "read_root_file", return_value=fixed.encode("ascii")):
            with self.assertRaises(ops.runtime.RuntimeErrorCode):
                self.database.execute(self.values, "SELECT 1;", None, backup_audit=True)
        for sql in (fixed + fixed,
                    "SELECT '{\"schema\":\"lagrange-daily-backup-db-v1\",\"schema\":\"wrong\"}';",
                    "SELECT '{\"schema\":\"wrong\"}';"):
            with self.subTest(case="closed response"), \
                    mock.patch.object(ops.install, "read_root_file", return_value=sql.encode("ascii")):
                with self.assertRaises((ops.runtime.RuntimeErrorCode, ops.install.DailyInstallError)):
                    self.database.backup_audit(self.values)
        self.assert_no_oneshot()


if __name__ == "__main__":
    unittest.main(verbosity=2)
