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
    @classmethod
    def command(cls, args, payload=None, *, timeout=120, allow_failure=False):
        result = subprocess.run(["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", *args],
            env=backup.ops.ENVIRONMENT, input=payload, capture_output=True, timeout=timeout, check=False)
        if result.returncode != 0 and not allow_failure:
            raise RuntimeError("DAILY_BACKUP_FIXTURE_COMMAND")
        return result

    @classmethod
    def sql(cls, text, *, role="postgres", timeout=120):
        return cls.command(["exec", "--interactive", "--user", "999:999", cls.server,
            "psql", "-X", "-q", "-A", "-t", "-U", role, "-d", "lagrange", "-v", "ON_ERROR_STOP=1"],
            text.encode(), timeout=timeout).stdout

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
        for name in ("daily-backup-verify.sh", "owner-beta-daily-backup-audit.sql"):
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
            "--env", "POSTGRES_DB=lagrange", "--env", "POSTGRES_PASSWORD_FILE=/run/test-secret",
            "--mount", "type=bind,src=" + str(secret) + ",dst=/run/test-secret,readonly", backup.POSTGRES
        ]).stdout.decode().strip()
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            if cls.command(["exec", cls.server, "pg_isready", "-U", "postgres", "-d", "lagrange"],
                           allow_failure=True).returncode == 0:
                break
            time.sleep(0.25)
        else:
            raise RuntimeError("DAILY_BACKUP_FIXTURE_DATABASE")
        roles = ("migration_owner", "app", "worker", "admin", "research_writer", "audit_writer", "daily_input_admitter")
        cls.sql("".join("CREATE ROLE " + role + " LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE "
                        "NOREPLICATION NOBYPASSRLS;\n" for role in roles) +
                "GRANT CONNECT ON DATABASE lagrange TO " + ",".join(roles) + ";\n"
                "GRANT USAGE,CREATE ON SCHEMA public TO migration_owner;\n"
                "GRANT USAGE ON SCHEMA public TO " + ",".join(roles[1:]) + ";\n")
        migrations = sorted((ROOT / "migrations").glob("*.up.sql"))
        if len(migrations) != 62:
            raise RuntimeError("DAILY_BACKUP_FIXTURE_MIGRATION_INVENTORY")
        identity = cls.sql("SELECT current_user||':'||session_user;", role="migration_owner").strip()
        if identity != b"migration_owner:migration_owner":
            raise RuntimeError("DAILY_BACKUP_FIXTURE_ROLE")
        for path in migrations:
            cls.sql("BEGIN;\n" + path.read_text() + "\nCOMMIT;", role="migration_owner")
        # The normal SQLx metadata shape is included in the dump. Checksums are
        # the real embedded migration bytes; the dedicated lane tests SQLx itself.
        cls.sql("CREATE TABLE public._sqlx_migrations(version bigint PRIMARY KEY,description text NOT NULL,"
                "installed_on timestamptz NOT NULL DEFAULT now(),success boolean NOT NULL,"
                "checksum bytea NOT NULL,execution_time bigint NOT NULL);" + "".join(
                    "INSERT INTO public._sqlx_migrations(version,description,success,checksum,execution_time) "
                    "VALUES(" + str(int(path.name[:4])) + ",'fixture',true,decode('" +
                    hashlib.sha384(path.read_bytes()).hexdigest() + "','hex'),0);" for path in migrations),
                role="migration_owner")
        cls.policy = (ROOT / "configs/evidence/owner-beta-daily-price-policy-v1.json").read_bytes()
        cls.policy_hash = "sha256:" + hashlib.sha256(cls.policy).hexdigest()
        policy_sql = cls.policy.decode().replace("'", "''")
        cls.sql("INSERT INTO users(id,issuer,subject,email) VALUES('" + OWNER +
            "','backup-fixture','backup-fixture','backup-fixture@example.invalid');"
            "INSERT INTO owner_beta_daily_control(owner_user_id,policy_sha256,policy_json,"
            "entitlement_reference,entitlement_sha256,first_auto_session,max_jobs_per_owner,lease_seconds,backoff_seconds)"
            " VALUES('" + OWNER + "','" + cls.policy_hash + "','" + policy_sql +
            "','backup-fixture','sha256:" + "b" * 64 + "','2026-08-29',4,60,1);")
        cls.audit = json.loads(cls.sql((ROOT / "deploy/db/owner-beta-daily-backup-audit.sql").read_text()))

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

    def test_actual_full_schema_encrypted_files_and_networkless_database_restore(self):
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

    def test_disk_floor_failure_stops_before_dump_or_container_creation(self):
        with mock.patch.object(backup.shutil, "disk_usage",
                               return_value=types.SimpleNamespace(free=self.values["MIN_FREE_BYTES"] - 1)):
            with self.assertRaises(backup.runtime.RuntimeErrorCode):
                self.capture()
        self.assertEqual(self.docker.verified, [])
        self.assertEqual(list(self.backups.iterdir()), [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
