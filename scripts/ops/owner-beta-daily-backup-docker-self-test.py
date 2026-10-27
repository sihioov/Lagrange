#!/usr/bin/env python3
"""Disposable full-schema dump/encryption/file/isolated-restore checks.

Root and Docker are required, never skipped. Migrations are applied as the
actual migration_owner login in one disposable database; this is not a SQLx
upgrade-lane replacement. No provider, installed release, or production path.
"""
from __future__ import annotations
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time
import types
import unittest
import uuid
from unittest import mock

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


backup = load("daily_backup_docker_tested", ROOT / "scripts/ops/lib/owner-beta-daily-backup.py")
fixture = load("daily_backup_file_fixture", ROOT / "scripts/ops/owner-beta-daily-backup-self-test.py")
COMMIT = "a" * 40
OWNER = "00000000-0000-4000-8000-000000000001"


class RecordingDocker(backup.runtime.Docker):
    def __init__(self):
        super().__init__()
        self.verified = []

    def object(self, kind, identity, timeout=15):
        value = super().object(kind, identity, timeout)
        if kind == "container" and (value.get("Config", {}).get("Labels") or {}).get(
                "io.lagrange.daily-backup-verify"):
            self.verified.append(value)  # synthetic fixture; retained in memory only
        return value


class DailyBackupDockerTests(unittest.TestCase):
    admin_role = "lagrange"

    @classmethod
    def command(cls, args, payload=None, *, timeout=120, allow_failure=False):
        result = subprocess.run(["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", *args],
            env=backup.ops.ENVIRONMENT, input=payload, capture_output=True, timeout=timeout, check=False)
        if result.returncode != 0 and not allow_failure:
            raise RuntimeError("DAILY_BACKUP_FIXTURE_COMMAND")
        return result

    @classmethod
    def sql(cls, text, *, role=None, timeout=120, stage="statement"):
        role = cls.admin_role if role is None else role
        if stage not in {"roles", "identity", "migration", "ledger", "seed", "audit", "statement"}:
            raise RuntimeError("DAILY_BACKUP_FIXTURE_STAGE")
        result = cls.command(["exec", "--interactive", "--user", "999:999", cls.server,
            "psql", "-X", "-q", "-A", "-t", "-U", role, "-d", "lagrange",
            "-v", "ON_ERROR_STOP=1", "-v", "VERBOSITY=sqlstate"],
            text.encode(), timeout=timeout, allow_failure=True)
        if result.returncode != 0:
            raise RuntimeError(f"DAILY_BACKUP_FIXTURE_SQL:{stage}:{result.returncode}:{cls.sqlstate(result)}")
        return result.stdout

    @staticmethod
    def sqlstate(result):
        # Never include raw stderr, SQL, mounted secret or database contents.
        match = re.search(rb"(?:ERROR|FATAL):\s+([0-9A-Z]{5})(?:\s|$)", result.stderr)
        return match[1].decode("ascii") if match else "UNKNOWN"

    @classmethod
    def database_probe(cls, database="lagrange", *, timeout=5):
        return cls.command(["exec", "--interactive", "--user", "999:999", cls.server,
            "sh", "-c", 'set -eu; PGPASSWORD="$(cat /run/test-secret)"; '
            'export PGPASSWORD PGCONNECT_TIMEOUT=2; '
            'exec psql -X -w -qAt -h 127.0.0.1 -U "$2" -d "$1" '
            '-v ON_ERROR_STOP=1 -v VERBOSITY=sqlstate', "fixture-db-probe", database, cls.admin_role],
            b"SELECT current_database()||':'||current_user||':'||session_user||':'||"
            b"(inet_server_addr() IS NOT NULL)::text;\n", timeout=timeout, allow_failure=True)

    @classmethod
    def wait_for_database(cls):
        deadline = time.monotonic() + 90
        diagnostic = "UNKNOWN"
        while (remaining := deadline - time.monotonic()) > 0:
            try:
                result = cls.database_probe(timeout=min(5, remaining))
                expected = ("lagrange:" + cls.admin_role + ":" + cls.admin_role + ":true").encode()
                if result.returncode == 0 and result.stdout.strip() == expected:
                    return
                diagnostic = cls.sqlstate(result) if result.returncode else "IDENTITY"
            except subprocess.TimeoutExpired:
                diagnostic = "TIMEOUT"
            time.sleep(min(0.25, max(0, deadline - time.monotonic())))
        raise RuntimeError("DAILY_BACKUP_FIXTURE_DATABASE:" + diagnostic)

    @classmethod
    def setUpClass(cls):
        if os.geteuid() != 0 or sys.platform != "linux":
            raise RuntimeError("DAILY_BACKUP_ACTUAL_ROOT_REQUIRED")
        cls.directory = Path(tempfile.mkdtemp(prefix="lagrange-backup-docker-", dir="/var/lib"))
        cls.network = cls.server = None
        cls.project = "daily-backup-" + uuid.uuid4().hex[:16]
        cls.addClassCleanup(cls.cleanup)
        cls.release = cls.directory / "releases" / COMMIT
        (cls.release / "deploy/db").mkdir(parents=True)
        for name in ("daily-backup-verify.sh", "owner-beta-daily-backup-audit.sql",
                     "owner-beta-daily-control.sql", "owner-beta-daily-control-status.sql"):
            shutil.copyfile(ROOT / "deploy/db" / name, cls.release / "deploy/db" / name)
        cls.network = cls.command(["network", "create", "--internal", "--label",
            "com.docker.compose.project=" + cls.project, "--label",
            "com.docker.compose.network=owner-beta-db", cls.project + "_owner-beta-db"]).stdout.decode().strip()
        # A random synthetic bootstrap secret is file-mounted, never printed or
        # copied into the backup. Local fixture psql uses the image's local socket.
        secret = cls.directory / "postgres-secret"
        secret.write_text(uuid.uuid4().hex + uuid.uuid4().hex)
        secret.chmod(0o400)
        os.chown(secret, 999, 999)
        cls.server = cls.command(["run", "--detach", "--name", cls.project + "-postgres",
            "--network", cls.network, "--network-alias", "postgres", "--label",
            "com.docker.compose.project=" + cls.project, "--label", "com.docker.compose.service=postgres",
            "--env", "POSTGRES_USER=lagrange", "--env", "POSTGRES_DB=lagrange",
            "--env", "POSTGRES_PASSWORD_FILE=/run/test-secret",
            "--mount", "type=bind,src=" + str(secret) + ",dst=/run/test-secret,readonly", backup.POSTGRES
        ]).stdout.decode().strip()
        cls.wait_for_database()
        roles = ("migration_owner", "app", "worker", "admin", "research_writer", "audit_writer", "daily_input_admitter")
        cls.sql("".join("CREATE ROLE " + role + " LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE "
                        "NOREPLICATION NOBYPASSRLS;\n" for role in roles) +
                "GRANT CONNECT ON DATABASE lagrange TO " + ",".join(roles) + ";\n"
                "GRANT USAGE,CREATE ON SCHEMA public TO migration_owner;\n"
                "GRANT USAGE ON SCHEMA public TO " + ",".join(roles[1:]) + ";\n", stage="roles")
        migrations = sorted((ROOT / "migrations").glob("*.up.sql"))
        if len(migrations) != 64:
            raise RuntimeError("DAILY_BACKUP_FIXTURE_MIGRATION_INVENTORY")
        identity = cls.sql("SELECT current_user||':'||session_user;", role="migration_owner", stage="identity").strip()
        if identity != b"migration_owner:migration_owner":
            raise RuntimeError("DAILY_BACKUP_FIXTURE_ROLE")
        cls.status_sql = (ROOT / "deploy/db/owner-beta-daily-control-status.sql").read_text()
        for path in migrations:
            script = path.read_text()
            # Match the migration's existing SQLx execution contract. Concurrent
            # indexes must be the sole statement outside a transaction; normal
            # migrations still need their transaction and SET LOCAL semantics.
            if not script.startswith("-- no-transaction\n"):
                script = "BEGIN;\n" + script + "\nCOMMIT;"
            cls.sql(script, role="migration_owner", stage="migration")
            if int(path.name[:4]) == 59:
                cls.status_schema59 = cls.sql(cls.status_sql, role="migration_owner").strip()
        # The normal SQLx metadata shape is included in the dump. Checksums are
        # the real embedded migration bytes; the dedicated lane tests SQLx itself.
        cls.sql("CREATE TABLE public._sqlx_migrations(version bigint PRIMARY KEY,description text NOT NULL,"
                "installed_on timestamptz NOT NULL DEFAULT now(),success boolean NOT NULL,"
                "checksum bytea NOT NULL,execution_time bigint NOT NULL);" + "".join(
                    "INSERT INTO public._sqlx_migrations(version,description,success,checksum,execution_time) "
                    "VALUES(" + str(int(path.name[:4])) + ",'fixture',true,decode('" +
                    hashlib.sha384(path.read_bytes()).hexdigest() + "','hex'),0);" for path in migrations),
                role="migration_owner", stage="ledger")
        cls.policy = (ROOT / "configs/evidence/owner-beta-daily-price-policy-v1.json").read_bytes()
        cls.policy_hash = "sha256:" + hashlib.sha256(cls.policy).hexdigest()
        cls.status_schema62 = cls.sql(cls.status_sql, role="migration_owner").strip()
        cls.sql("INSERT INTO users(id,issuer,subject,email) VALUES('" + OWNER +
            "','backup-fixture','backup-fixture','backup-fixture@example.invalid');"
            "INSERT INTO roles(id) VALUES('owner') ON CONFLICT DO NOTHING;"
            "INSERT INTO user_roles(user_id,role_id) VALUES('" + OWNER + "','owner');"
            "INSERT INTO data_entitlements(contract_document_sha256,contract_reference,status,"
            "covered_datasets,covered_uses,effective_from,effective_until,managed_by) VALUES('" + "e" * 64 +
            "','fixture-only-not-an-activation','ACTIVE','[\"krx_eod_bars\"]','[\"recommendation\"]',"
            "'2016-08-29','2099-12-31','" + OWNER + "');", stage="seed")
        config_source = cls.directory / "daily-config.json"
        config_source.write_bytes((ROOT / "tests/fixtures/owner-beta-daily-install/config.json").read_bytes())
        config_source.chmod(0o600)
        policy_source = cls.directory / "daily-policy.json"
        policy_source.write_bytes(cls.policy)
        policy_source.chmod(0o644)
        backup.install.stage_config(str(cls.release), str(config_source), str(policy_source), COMMIT)
        prepare = backup.install.render_control(str(cls.release), COMMIT, "prepare", "4", "60", "1")
        cls.first_prepare = cls.sql(prepare, role="migration_owner").strip()
        cls.status_prepared = cls.sql(cls.status_sql, role="migration_owner").strip()
        cls.audit = json.loads(cls.sql((ROOT / "deploy/db/owner-beta-daily-backup-audit.sql").read_text(), role="migration_owner", stage="audit"))

    @classmethod
    def cleanup(cls):
        failures = []
        def remove(args):
            try:
                if cls.command(args, allow_failure=True).returncode != 0:
                    failures.append("cleanup")
            except Exception:
                failures.append("cleanup")
        try:
            leftover = cls.command(["ps", "--all", "--quiet", "--no-trunc", "--filter",
                "label=io.lagrange.daily-backup-project=" + cls.project]).stdout.decode().splitlines()
            for identity in leftover:
                if backup.runtime.CONTAINER.fullmatch(identity):
                    remove(["rm", "--force", "--volumes", identity])
                else:
                    failures.append("identity")
        except Exception:
            failures.append("enumeration")
        if cls.server:
            remove(["rm", "--force", "--volumes", cls.server])
        if cls.network:
            remove(["network", "rm", cls.network])
        shutil.rmtree(cls.directory)
        if failures:
            raise RuntimeError("DAILY_BACKUP_FIXTURE_CLEANUP")

    def setUp(self):
        self.fixture = fixture.DailyBackupFilesTests(methodName="test_all_classes_bytes_modes_and_empty_files_round_trip")
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.directory = self.fixture.directory
        shutil.rmtree(self.directory / "daily/admitted/hash")
        (self.directory / "daily/frontiers").mkdir(mode=0o750)
        os.chown(self.directory / "daily/frontiers", 10003, 10001)
        self.fixture.put("config/policy.json", self.policy).chmod(0o440)
        self.backups = self.directory / "backups"
        self.backups.mkdir(mode=0o700)
        self.key = self.directory / "key"
        self.key.write_bytes(b"9" * 64)
        self.key.chmod(0o600)
        self.docker = RecordingDocker()
        self.context = types.SimpleNamespace(commit=COMMIT, release=str(type(self).release))
        self.binding = types.SimpleNamespace(project=type(self).project, database="lagrange",
                                             config_sha256="sha256:" + "c" * 64)
        self.values = {"BACKUP_ROOT": str(self.backups), "KEY_FILE": str(self.key),
                       "MAX_TOTAL_BYTES": 64 * 1024**2, "MIN_FREE_BYTES": 16 * 1024**2,
                       "RETENTION_DAYS": 35, "MIN_KEEP": 1}
        self.storage = backup.Storage(self.values, self.docker, self.context, self.binding)

    def capture(self, database=None):
        with backup.files.Snapshot(self.fixture.roots, self.storage.limits) as snapshot:
            with self.storage.capture(snapshot, type(self).audit if database is None else database) as (source, final, name):
                self.storage.publish(source, final, name)
        return final

    def assert_no_verifier(self):
        self.assertEqual(self.command(["ps", "--all", "--quiet", "--filter",
            "label=io.lagrange.daily-backup-project=" + self.binding.project]).stdout.strip(), b"")

    def test_actual_database_probe_requires_target_sql_and_keeps_errors_closed(self):
        missing = "fixture_database_that_does_not_exist"
        accepting = self.command(["exec", self.server, "pg_isready", "-h", "127.0.0.1",
            "-U", "postgres", "-d", missing], allow_failure=True, timeout=5)
        self.assertEqual(accepting.returncode, 0)
        self.assertNotEqual(self.database_probe(missing).returncode, 0)
        ready = self.database_probe()
        self.assertEqual(ready.returncode, 0)
        self.assertEqual(ready.stdout.strip(), b"lagrange:lagrange:lagrange:true")
        with self.assertRaisesRegex(RuntimeError, r"^DAILY_BACKUP_FIXTURE_SQL:statement:3:42P01$"):
            self.sql('SELECT * FROM "fixture_private_sql_diagnostic";')

    def test_actual_schema59_and_62_bootstrap_status_and_first_prepare(self):
        self.assertEqual(self.status_schema59, b"DAILY_CONTROL_STATUS: ABSENT_SCHEMA")
        self.assertEqual(self.status_schema62, b"DAILY_CONTROL_STATUS: ABSENT_CONTROL")
        self.assertEqual(self.first_prepare, b"DAILY_CONTROL: DISABLED")
        self.assertEqual(self.status_prepared, b"DAILY_CONTROL_STATUS: DISABLED")
        observed = self.sql("SELECT relrowsecurity::text||':'||relforcerowsecurity::text||':'||"
            "pg_get_userbyid(relowner)||':'||pg_catalog.row_security_active(oid)::text "
            "FROM pg_class WHERE oid='public.users'::regclass;", role="migration_owner")
        self.assertEqual(observed.strip(), b"true:false:migration_owner:false")

    def test_live_dump_uses_existing_admin_without_creating_postgres_role(self):
        self.assertEqual(self.sql("SELECT current_user||':'||session_user||':'||"
            "(NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='postgres'))::text;"),
            b"lagrange:lagrange:true\n")
        before = self.sql("SELECT rolname||':'||rolsuper::text||':'||rolcanlogin::text "
                          "FROM pg_roles ORDER BY rolname;")
        with self.storage.temporary(".daily-verify-") as stage:
            target = stage / "live.dump"
            self.storage.dump(target)
            self.assertTrue(target.read_bytes().startswith(b"PGDMP"))
        self.assertEqual(self.sql("SELECT rolname||':'||rolsuper::text||':'||rolcanlogin::text "
                                 "FROM pg_roles ORDER BY rolname;"), before)
        self.assert_no_verifier()

    def test_separate_tool_verifier_restores_without_editing_application(self):
        application = type(self).release / "deploy/db"
        originals = {name: ((application / name).read_bytes(), (application / name).stat().st_ino)
                     for name in ("daily-backup-verify.sh", "owner-beta-daily-backup-audit.sql")}
        tool = self.directory / "tool"
        (tool / "deploy/db").mkdir(parents=True)
        for name, (data, _) in originals.items():
            (tool / "deploy/db" / name).write_bytes(data)
            (tool / "deploy/db" / name).chmod(0o644)
        self.storage.verifier_root = str(tool)
        final = self.capture()
        self.assertEqual(self.storage.verify(final), {"extra_admissions": [], "extra_frontiers": []})
        for name, original in originals.items():
            self.assertEqual(((application / name).read_bytes(), (application / name).stat().st_ino),
                             original)
        for value in self.docker.verified:
            mounts = {mount["Destination"]: mount["Source"] for mount in value["Mounts"]}
            self.assertEqual(mounts["/verify.sh"], str(tool / "deploy/db/daily-backup-verify.sh"))
            self.assertEqual(mounts["/verify.sql"], str(tool / "deploy/db/owner-beta-daily-backup-audit.sql"))
        self.assert_no_verifier()

    def test_actual_full_schema_encrypted_files_and_networkless_database_restore(self):
        migrations = sorted((ROOT / "migrations").glob("*.up.sql"))
        self.assertEqual(len(migrations), 64)
        ledger = self.sql("SELECT version::text||':'||encode(checksum,'hex')||':'||success::text "
                          "FROM public._sqlx_migrations ORDER BY version;", role="migration_owner")
        self.assertEqual(ledger.decode("ascii").splitlines(), [
            str(int(path.name[:4])) + ":" + hashlib.sha384(path.read_bytes()).hexdigest() + ":true"
            for path in migrations])
        expected_indexes = {
            "data_batches_source_file_uq": ("data_batches", "true"),
            "trading_calendar_versions_source_lookup_idx": ("trading_calendar_versions", "false"),
            "recommendation_runs_job_id_uq": ("recommendation_runs", "true"),
            "recommendation_runs_scheduled_identity_uq": ("recommendation_runs", "true"),
            "recommendation_items_run_instrument_key": ("recommendation_items", "true"),
            "target_portfolios_one_per_run": ("target_portfolios", "true"),
            "jobs_typed_claim_idx": ("jobs", "false"),
        }
        # Observe the real catalog after every migration, including 0030's
        # constraint attachment. A skipped or invalid concurrent build cannot
        # be hidden by writing a synthetic SQLx ledger row.
        names = ",".join("'" + name + "'" for name in expected_indexes)
        indexes = self.sql("SELECT i.relname||':'||t.relname||':'||pg_get_userbyid(i.relowner)||':'||"
            "x.indisvalid::text||':'||x.indisready::text||':'||x.indislive::text||':'||x.indisunique::text "
            "FROM pg_catalog.pg_index x JOIN pg_catalog.pg_class i ON i.oid=x.indexrelid "
            "JOIN pg_catalog.pg_class t ON t.oid=x.indrelid "
            "JOIN pg_catalog.pg_namespace n ON n.oid=i.relnamespace "
            "WHERE n.nspname='public' AND i.relname IN (" + names + ") ORDER BY i.relname;",
            role="migration_owner")
        self.assertEqual(indexes.decode("ascii").splitlines(), [
            name + ":" + table + ":migration_owner:true:true:true:" + unique
            for name, (table, unique) in sorted(expected_indexes.items())])
        final = self.capture()
        self.assertEqual(self.storage.verify(final), {"extra_admissions": [], "extra_frontiers": []})
        self.assertEqual({p.name for p in final.iterdir()}, {*backup.MEMBERS, "metadata.json", "COMPLETE"})
        for member in backup.MEMBERS:
            data = (final / member).read_bytes()
            self.assertTrue(data.startswith(b"Salted__"))
            for forbidden in (self.policy, b"curated\0fixture", b"approved-base-fixture", b"9" * 64):
                self.assertNotIn(forbidden, data)
        self.assertTrue(self.docker.verified)
        for value in self.docker.verified:
            self.assertEqual(value["HostConfig"]["NetworkMode"], "none")
            self.assertEqual(value["Config"]["User"], "999:999")
            sources = {m.get("Source", "") for m in value.get("Mounts", [])}
            self.assertNotIn(str(type(self).directory / "postgres-secret"), sources)
        self.assert_no_verifier()

    def test_cipher_corruption_is_rejected_before_another_restore_container(self):
        final = self.capture()
        before = len(self.docker.verified)
        path = final / "postgres.dump.enc"
        data = bytearray(path.read_bytes())
        data[-1] ^= 1
        path.write_bytes(data)
        with self.assertRaises(backup.runtime.RuntimeErrorCode):
            self.storage.verify(final)
        self.assertEqual(len(self.docker.verified), before)
        self.assert_no_verifier()

    def test_wrong_key_never_accepts_or_leaves_plaintext_staging(self):
        final = self.capture()
        self.key.write_bytes(b"8" * 64)
        with self.assertRaises(Exception):
            self.storage.verify(final)
        self.assertFalse(any(p.name.startswith(".daily-") for p in self.backups.iterdir()))
        self.assert_no_verifier()

    def test_restored_database_must_match_the_captured_audit(self):
        changed = json.loads(json.dumps(type(self).audit))
        changed["control"]["activation_epoch"] += 1
        with self.assertRaises(backup.runtime.RuntimeErrorCode):
            self.capture(changed)
        self.assertEqual(list(self.backups.iterdir()), [])
        self.assert_no_verifier()

    def test_invalid_dump_is_a_real_restore_failure_and_owned_container_is_removed(self):
        with self.storage.temporary(".invalid-") as temporary:
            dump = temporary / "invalid.dump"
            dump.write_bytes(b"not a PostgreSQL archive")
            dump.chmod(0o600)
            with self.assertRaises(backup.runtime.RuntimeErrorCode):
                self.storage.restore_database(dump, temporary)
        self.assert_no_verifier()

    def test_lost_create_reply_recovers_only_the_recorded_owned_verifier(self):
        original = self.docker.call
        created = []
        def lose_reply(args, timeout=15):
            if args[0] == "create":
                intents = list(self.backups.glob("daily-verifier-*.json"))
                self.assertEqual(len(intents), 1)
                intent = json.loads(intents[0].read_bytes())
                self.assertEqual(args[args.index("--name") + 1], intent["name"])
                self.assertTrue(Path(intent["staging"]).is_dir())
                identity = original(args, timeout).strip()
                self.assertRegex(identity, r"^[0-9a-f]{64}$")
                created.append(identity)
                raise backup.runtime.RuntimeErrorCode("TEST_CREATE_REPLY_LOST")
            return original(args, timeout)
        try:
            with mock.patch.object(self.docker, "call", side_effect=lose_reply):
                with self.assertRaisesRegex(backup.runtime.RuntimeErrorCode, "^TEST_CREATE_REPLY_LOST$"):
                    with self.storage.temporary(".daily-verify-") as stage:
                        dump = stage / "postgres.dump"
                        dump.write_bytes(b"invalid archive; verifier must never start")
                        self.storage.restore_database(dump, stage)
            self.assertEqual(len(created), 1)
            self.assertEqual(list(self.backups.iterdir()), [])
            self.assert_no_verifier()
        finally:
            for identity in created:
                remaining = self.command(["ps", "--all", "--quiet", "--no-trunc",
                    "--filter", "id=" + identity]).stdout.decode().splitlines()
                self.assertIn(remaining, ([], [identity]))
                if remaining:
                    self.command(["rm", "--force", "--volumes", identity])

    def test_unobservable_lost_create_retains_intent_copy_and_blocks_new_clone(self):
        original = self.docker.call
        created = []
        def lose_reply_and_hide(args, timeout=15):
            if args[0] == "create":
                identity = original(args, timeout).strip()
                self.assertRegex(identity, r"^[0-9a-f]{64}$")
                created.append(identity)
                raise backup.runtime.RuntimeErrorCode("TEST_CREATE_REPLY_LOST")
            if args[0] == "ps" and any(arg.startswith("label=io.lagrange.daily-backup-verify=") for arg in args):
                return ""  # Simulate the daemon not yet exposing its accepted create.
            return original(args, timeout)
        try:
            with mock.patch.object(self.docker, "call", side_effect=lose_reply_and_hide):
                with self.assertRaisesRegex(backup.runtime.RuntimeErrorCode, "^TEST_CREATE_REPLY_LOST$"):
                    with self.storage.temporary(".daily-verify-") as stage:
                        dump = stage / "postgres.dump"
                        dump.write_bytes(b"retained synthetic dump")
                        self.storage.restore_database(dump, stage)
                self.assertEqual(dump.read_bytes(), b"retained synthetic dump")
                intents = list(self.backups.glob("daily-verifier-*.json"))
                self.assertEqual(len(intents), 1)
                self.assertEqual(json.loads(intents[0].read_bytes())["staging"], str(stage))
                with self.assertRaisesRegex(backup.runtime.RuntimeErrorCode, "^DAILY_BACKUP_VERIFY_PENDING$"):
                    self.storage.restore_database(dump, stage)
                with self.assertRaisesRegex(backup.runtime.RuntimeErrorCode, "^DAILY_BACKUP_VERIFY_PENDING$"):
                    with self.storage.temporary(".daily-verify-"):
                        self.fail("pending verifier admitted another staging tree")
                self.assertEqual(len(created), 1)
        finally:
            # Test-owned disposable clone only. Remove before fixture teardown
            # can delete the retained copy; production never force-removes it.
            for identity in created:
                self.command(["rm", "--force", "--volumes", identity])
        self.assert_no_verifier()

    def test_cleanup_inspection_failure_preserves_staging_and_blocks_retry(self):
        original = self.docker.object
        observed = []
        def lose_cleanup(kind, identity, timeout=15):
            if kind == "container":
                observed.append(identity)
                if len(observed) > 1:
                    raise backup.runtime.RuntimeErrorCode("TEST_CLEANUP_OBSERVATION_LOST")
            return original(kind, identity, timeout)
        try:
            with mock.patch.object(self.docker, "object", side_effect=lose_cleanup):
                with self.assertRaisesRegex(backup.runtime.RuntimeErrorCode, "^TEST_CLEANUP_OBSERVATION_LOST$"):
                    with self.storage.temporary(".daily-verify-") as stage:
                        dump = stage / "postgres.dump"
                        dump.write_bytes(b"invalid synthetic PostgreSQL archive")
                        self.storage.restore_database(dump, stage)
            self.assertEqual(len(set(observed)), 1)
            self.assertTrue(dump.exists())
            self.assertTrue((stage / "postgres").is_dir())
            intents = list(self.backups.glob("daily-verifier-*.json"))
            self.assertEqual(len(intents), 1)
            self.assertEqual(json.loads(intents[0].read_bytes())["staging"], str(stage))
            with self.assertRaisesRegex(backup.runtime.RuntimeErrorCode, "^DAILY_BACKUP_VERIFY_PENDING$"):
                self.storage.restore_database(dump, stage)
        finally:
            for identity in set(observed):
                self.command(["rm", "--force", "--volumes", identity])
        self.assert_no_verifier()

    def test_disk_floor_failure_stops_before_dump_or_container_creation(self):
        with mock.patch.object(backup.shutil, "disk_usage",
                               return_value=types.SimpleNamespace(free=self.values["MIN_FREE_BYTES"] - 1)):
            with self.assertRaises(backup.runtime.RuntimeErrorCode):
                self.capture()
        self.assertEqual(self.docker.verified, [])
        self.assertEqual(list(self.backups.iterdir()), [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
