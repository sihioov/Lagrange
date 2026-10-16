#!/usr/bin/env python3
"""Actual schema59 cold-cluster recovery in disposable, networkless containers.

Requires Linux root and Docker; never skips or opens installed production data.
SQL fixtures use the real old migrations. Existing SQLx upgrade-lane evidence
remains separate from this fixture's manually bound migration ledger.
"""
from __future__ import annotations

import copy
import hashlib
import importlib.util
import os
from pathlib import Path
import shutil
import sys
import tempfile
import types
import unittest
import uuid

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


recovery = load("preupgrade_docker_tested", ROOT / "scripts/ops/lib/production-preupgrade-recovery.py")
helpers = load("preupgrade_docker_helpers", ROOT / "scripts/ops/owner-beta-daily-backup-docker-self-test.py")
backup, files = recovery.backup, recovery.files


class PreupgradeDockerTests(unittest.TestCase):
    command = classmethod(helpers.DailyBackupDockerTests.command.__func__)
    sql = classmethod(helpers.DailyBackupDockerTests.sql.__func__)
    sqlstate = staticmethod(helpers.DailyBackupDockerTests.sqlstate)
    database_probe = classmethod(helpers.DailyBackupDockerTests.database_probe.__func__)
    wait_for_database = classmethod(helpers.DailyBackupDockerTests.wait_for_database.__func__)

    @classmethod
    def setUpClass(cls):
        if os.geteuid() != 0 or sys.platform != "linux":
            raise RuntimeError("PREUPGRADE_FIXTURE_ROOT_REQUIRED")
        cls.directory = Path(tempfile.mkdtemp(prefix="lagrange-preupgrade-docker-", dir="/var/lib"))
        cls.project = "preupgrade-" + uuid.uuid4().hex[:16]
        cls.server = cls.volume = None
        cls.addClassCleanup(cls.cleanup)
        cls.docker = backup.runtime.Docker()
        cls.release = cls.directory / "release"
        (cls.release / "deploy/db").mkdir(parents=True)
        for name in ("preupgrade-recovery-verify.sh", "preupgrade-recovery-audit.sql"):
            shutil.copyfile(ROOT / "deploy/db" / name, cls.release / "deploy/db" / name)
        cls.volume = cls.command(["volume", "create", "--label",
            "io.lagrange.preupgrade-fixture=" + cls.project, cls.project + "-data"]).stdout.decode().strip()
        secret = cls.directory / "secret"
        secret.write_text(uuid.uuid4().hex + uuid.uuid4().hex)
        secret.chmod(0o400)
        os.chown(secret, 999, 999)
        cls.server = cls.command(["run", "--detach", "--name", cls.project + "-source", "--network=none",
            "--label", "io.lagrange.preupgrade-fixture=" + cls.project,
            "--env", "POSTGRES_DB=lagrange", "--env", "POSTGRES_PASSWORD_FILE=/run/test-secret",
            "--mount", "type=volume,src=" + cls.volume + ",dst=/var/lib/postgresql",
            "--mount", "type=bind,src=" + str(secret) + ",dst=/run/test-secret,readonly", backup.POSTGRES
        ]).stdout.decode().strip()
        cls.wait_for_database()
        roles = ("migration_owner", "app", "worker", "admin", "research_writer", "audit_writer")
        cls.sql("".join("CREATE ROLE " + role + " LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE "
                "NOREPLICATION NOBYPASSRLS;\n" for role in roles) +
                "GRANT CONNECT ON DATABASE lagrange TO " + ",".join(roles) + ";\n"
                "GRANT USAGE,CREATE ON SCHEMA public TO migration_owner;\n"
                "GRANT USAGE ON SCHEMA public TO " + ",".join(roles[1:]) + ";\n", stage="roles")
        migrations = sorted(p for p in (ROOT / "migrations").glob("*.up.sql") if int(p.name[:4]) <= 59)
        if len(migrations) != 59:
            raise RuntimeError("PREUPGRADE_FIXTURE_MIGRATIONS")
        for path in migrations:
            statement = path.read_text()
            if not statement.startswith("-- no-transaction\n"):
                statement = "BEGIN;\n" + statement + "\nCOMMIT;"
            cls.sql(statement, role="migration_owner", stage="migration")
        cls.migrations = [{"version": int(p.name[:4]), "checksum": hashlib.sha384(p.read_bytes()).hexdigest(),
                           "success": True} for p in migrations]
        cls.sql("CREATE TABLE public._sqlx_migrations(version bigint PRIMARY KEY,description text NOT NULL,"
            "installed_on timestamptz NOT NULL DEFAULT now(),success boolean NOT NULL,checksum bytea NOT NULL,"
            "execution_time bigint NOT NULL);" + "".join(
                "INSERT INTO public._sqlx_migrations(version,description,success,checksum,execution_time) "
                "VALUES(" + str(row["version"]) + ",'fixture',true,decode('" + row["checksum"] + "','hex'),0);"
                for row in cls.migrations), role="migration_owner", stage="ledger")
        cls.sql("CREATE ROLE cold_fixture_reader LOGIN NOINHERIT NOBYPASSRLS;"
                "GRANT worker TO cold_fixture_reader;"
                "CREATE TABLE public.cold_fixture_rows(id bigint PRIMARY KEY,body text NOT NULL);"
                "ALTER TABLE public.cold_fixture_rows OWNER TO migration_owner;"
                "INSERT INTO public.cold_fixture_rows VALUES(1,'retained-data'),(2,'restricted-data');"
                "ALTER TABLE public.cold_fixture_rows ENABLE ROW LEVEL SECURITY;"
                "ALTER TABLE public.cold_fixture_rows FORCE ROW LEVEL SECURITY;"
                "CREATE POLICY cold_reader ON public.cold_fixture_rows TO cold_fixture_reader USING(id=1);"
                "GRANT SELECT ON public.cold_fixture_rows TO cold_fixture_reader;", stage="seed")
        cls.audit = backup.install.json_bytes(cls.sql((ROOT / "deploy/db/preupgrade-recovery-audit.sql").read_text(),
                                                       stage="audit"), maximum=1024 * 1024)
        recovery.checked_audit(cls.audit, database="lagrange", system_identifier=cls.audit["system_identifier"],
                               migrations=cls.migrations)
        cls.command(["stop", "--time", "30", cls.server], timeout=40)
        info = cls.docker.object("container", cls.server)
        volume = recovery.volume_info(cls.docker, cls.volume)
        if (volume.get("Labels") or {}).get("io.lagrange.preupgrade-fixture") != cls.project:
            raise RuntimeError("PREUPGRADE_FIXTURE_VOLUME_OWNER")
        cls.mountpoint = volume["Mountpoint"]
        cls.generation = (info["Created"], info["State"]["StartedAt"], info["State"]["FinishedAt"], info["RestartCount"])
        cls.image_id = info["Image"]
        cls.check_stopped()

    @classmethod
    def check_stopped(cls):
        recovery.stopped_container(cls.docker.object("container", cls.server), identity=cls.server,
            image_id=cls.image_id, volume=cls.volume, mountpoint=cls.mountpoint, generation=cls.generation)

    @classmethod
    def cleanup(cls):
        failures = []
        # Every name/ID here was freshly created by this fixture. Recheck its
        # ownership label before removal; a foreign replacement is an error.
        try:
            remaining = cls.command(["ps", "--all", "--quiet", "--no-trunc", "--filter",
                "label=io.lagrange.preupgrade-project=" + cls.project]).stdout.decode().splitlines()
            if remaining:
                failures.append("verifier-leftover")
            for identity in remaining:
                value = cls.docker.object("container", identity)
                if (value["Config"].get("Labels") or {}).get("io.lagrange.preupgrade-project") == cls.project:
                    cls.command(["rm", "--force", "--volumes", identity])
            # Also recover a create/run whose response was lost before its ID
            # could be assigned above. The fresh fixture label bounds discovery.
            sources = cls.command(["ps", "--all", "--quiet", "--no-trunc", "--filter",
                "label=io.lagrange.preupgrade-fixture=" + cls.project]).stdout.decode().splitlines()
            if len(sources) > 1:
                raise RuntimeError("source-count")
            for identity in sources:
                if not backup.runtime.CONTAINER.fullmatch(identity):
                    raise RuntimeError("identity")
                value = cls.docker.object("container", identity)
                if (value["Config"].get("Labels") or {}).get("io.lagrange.preupgrade-fixture") != cls.project:
                    raise RuntimeError("owner")
                cls.command(["rm", "--force", "--volumes", identity])
            volumes = cls.command(["volume", "ls", "--quiet", "--filter",
                "label=io.lagrange.preupgrade-fixture=" + cls.project]).stdout.decode().splitlines()
            if len(volumes) > 1:
                raise RuntimeError("volume-count")
            for name in volumes:
                value = recovery.volume_info(cls.docker, name)
                if (value.get("Labels") or {}).get("io.lagrange.preupgrade-fixture") != cls.project:
                    raise RuntimeError("owner")
                cls.command(["volume", "rm", name])
        except Exception:
            failures.append("cleanup")
        shutil.rmtree(cls.directory)
        if failures:
            raise RuntimeError("PREUPGRADE_FIXTURE_CLEANUP")

    def setUp(self):
        self.check_stopped()
        self.case = Path(tempfile.mkdtemp(prefix="case-", dir=type(self).directory))
        self.addCleanup(shutil.rmtree, self.case)
        self.roots = []
        for name in files.PREUPGRADE_CLASSES:
            if name == "postgres":
                path = Path(self.mountpoint)
                info = path.stat()
                self.roots.append(files.Root(name, str(path), info.st_uid, info.st_gid, info.st_mode & 0o7777))
            else:
                path = self.case / name
                path.mkdir(mode=0o750)
                owner = 10001 if name in {"raw", "curated", "base"} else 0
                os.chown(path, owner, 10001 if owner else 0)
                payload = path / "fixture"
                payload.write_bytes(("synthetic-" + name).encode())
                payload.chmod(0o600)
                os.chown(payload, owner, 10001 if owner else 0)
                self.roots.append(files.Root(name, str(path), owner, 10001 if owner else 0, 0o750))
        root = self.case / "backups"
        root.mkdir(mode=0o700)
        key = self.case / "key"
        key.write_bytes(uuid.uuid4().hex.encode() * 2)
        key.chmod(0o600)
        self.storage = recovery.ColdStorage({"BACKUP_ROOT": str(root), "KEY_FILE": str(key),
                "MAX_TOTAL_BYTES": 512 * 1024**2, "MIN_FREE_BYTES": 32 * 1024**2}, self.docker,
            types.SimpleNamespace(release=str(self.release)), types.SimpleNamespace(project=self.project, database="lagrange"))

    def restore_files(self, stage):
        plain, cipher, decrypted = stage / "files.bundle", stage / "files.enc", stage / "files.decrypted"
        with files.Snapshot(self.roots, self.storage.limits, profile="preupgrade") as snapshot:
            fd = os.open(plain, os.O_CREAT | os.O_EXCL | os.O_WRONLY | os.O_NOFOLLOW, 0o600)
            with os.fdopen(fd, "wb") as stream:
                snapshot.write(stream)
            manifest = backup.install.json_bytes(snapshot.manifest(), maximum=self.storage.limits.manifest_bytes)
            self.storage.crypt(plain, cipher)
            self.check_stopped()
        self.storage.crypt(cipher, decrypted, decrypt=True)
        restored = stage / "restored"
        restored.mkdir(mode=0o700)
        with decrypted.open("rb") as stream:
            self.assertEqual(files.restore_archive(stream, self.storage.limits, str(restored),
                             self.storage.reserve, profile="preupgrade"), manifest)
        return restored, {"files": manifest, "database": copy.deepcopy(self.audit)}

    def assert_no_verifier(self):
        self.assertEqual(self.command(["ps", "--all", "--quiet", "--filter",
            "label=io.lagrange.preupgrade-project=" + self.project]).stdout.strip(), b"")
        self.check_stopped()

    def test_real_schema59_encrypted_cluster_catalog_and_files_round_trip(self):
        with self.storage.temporary(".preupgrade-") as stage:
            restored, expected = self.restore_files(stage)
            actual = self.storage.verify_cluster(restored, expected)
            self.assertEqual(actual, self.audit)
            self.assertEqual(actual["migrations"], self.migrations)
            for name in ("raw", "curated", "base", "config"):
                self.assertEqual((restored / name / "fixture").read_bytes(), ("synthetic-" + name).encode())
            self.assert_no_verifier()
        self.assertEqual(list(Path(self.storage.root).iterdir()), [])

    def test_catalog_mismatch_is_a_real_restore_failure_with_owned_cleanup(self):
        with self.storage.temporary(".preupgrade-") as stage:
            restored, expected = self.restore_files(stage)
            expected["database"]["catalog_sha256"] = "sha256:" + "0" * 64
            with self.assertRaises(recovery.runtime.RuntimeErrorCode):
                self.storage.verify_cluster(restored, expected)
            self.assert_no_verifier()

    def test_corrupt_control_with_valid_archive_hash_fails_native_cluster_validation(self):
        with self.storage.temporary(".preupgrade-") as stage:
            restored, expected = self.restore_files(stage)
            control = restored / recovery.PGROOT / "global/pg_control"
            control.write_bytes(b"invalid-control" * 1024)
            roots = []
            for name in files.PREUPGRADE_CLASSES:
                path = restored / name
                info = path.stat()
                roots.append(files.Root(name, str(path), info.st_uid, info.st_gid, info.st_mode & 0o7777))
            with files.Snapshot(roots, self.storage.limits, profile="preupgrade") as snapshot:
                expected["files"] = backup.install.json_bytes(snapshot.manifest(), maximum=self.storage.limits.manifest_bytes)
            with self.assertRaises(recovery.runtime.RuntimeErrorCode):
                self.storage.verify_cluster(restored, expected)
            self.assert_no_verifier()


if __name__ == "__main__":
    unittest.main(verbosity=2)
