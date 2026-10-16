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
import stat
import sys
import tempfile
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
        cls.cluster_observations = []
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
        cls.protect_fixture_volume_parent()

    @classmethod
    def protect_fixture_volume_parent(cls):
        """Establish the cold snapshot contract on this fixture's own volume."""
        before = recovery.volume_info(cls.docker, cls.volume)
        mountpoint = Path(cls.mountpoint)
        if (before["Mountpoint"] != cls.mountpoint or mountpoint.name != "_data"
                or mountpoint.parent.name != cls.volume
                or (before.get("Labels") or {}).get("io.lagrange.preupgrade-fixture") != cls.project):
            raise RuntimeError("PREUPGRADE_FIXTURE_VOLUME_OWNER")
        cls.check_stopped()
        # Hold the protected shared ancestor but change only the fresh labelled
        # volume's own parent, never Docker's shared volumes directory.
        shared = mountpoint.parent.parent
        shared_mode = stat.S_IMODE(shared.stat(follow_symlinks=False).st_mode)
        with files.open_root(files.Root("config", str(shared), 0, 0, shared_mode)) as ancestor:
            fd = os.open(cls.volume, files.DIRECTORY, dir_fd=ancestor)
            try:
                original = files.checked(fd, directory=True, owners={0}, groups={0})
                named = os.stat(cls.volume, dir_fd=ancestor, follow_symlinks=False)
                if (named.st_dev, named.st_ino) != (original.st_dev, original.st_ino):
                    raise RuntimeError("PREUPGRADE_FIXTURE_VOLUME_CHANGED")
                os.fchmod(fd, 0o700)
                protected = files.checked(fd, directory=True, owners={0}, groups={0})
                named = os.stat(cls.volume, dir_fd=ancestor, follow_symlinks=False)
                if ((protected.st_dev, protected.st_ino) != (original.st_dev, original.st_ino)
                        or (named.st_dev, named.st_ino) != (protected.st_dev, protected.st_ino)
                        or stat.S_IMODE(protected.st_mode) != 0o700):
                    raise RuntimeError("PREUPGRADE_FIXTURE_VOLUME_CHANGED")
                after = recovery.volume_info(cls.docker, cls.volume)
                changed = tuple(key for key in
                    ("Name", "Driver", "Scope", "Options", "Labels", "Mountpoint", "CreatedAt")
                    if after.get(key) != before.get(key))
                # Docker's local driver derives CreatedAt from this parent's
                # ctime. Our fchmod can change it without replacing the volume.
                # Report only closed field names, never inspected values.
                print("PREUPGRADE_FIXTURE_VOLUME_FIELDS:" + (",".join(changed) or "none"), flush=True)
                if any(key != "CreatedAt" for key in changed):
                    raise RuntimeError("PREUPGRADE_FIXTURE_VOLUME_CHANGED")
                cls.check_stopped()
                # Keep the owner descriptor through Docker/generation checks;
                # the name must still resolve to that same protected directory.
                final = files.checked(fd, directory=True, owners={0}, groups={0})
                named = os.stat(cls.volume, dir_fd=ancestor, follow_symlinks=False)
                if ((final.st_dev, final.st_ino, final.st_uid, final.st_gid, final.st_mode) !=
                        (protected.st_dev, protected.st_ino, protected.st_uid,
                         protected.st_gid, protected.st_mode)
                        or (named.st_dev, named.st_ino) != (final.st_dev, final.st_ino)):
                    raise RuntimeError("PREUPGRADE_FIXTURE_VOLUME_CHANGED")
            finally:
                os.close(fd)
        print(f"PREUPGRADE_FIXTURE_PARENT:uid=0:gid=0:before={stat.S_IMODE(original.st_mode):04o}:after=0700",
              flush=True)

    @classmethod
    def check_stopped(cls):
        recovery.stopped_container(cls.docker.object("container", cls.server), identity=cls.server,
            image_id=cls.image_id, volume=cls.volume, mountpoint=cls.mountpoint, generation=cls.generation)

    @classmethod
    def cleanup(cls):
        # Print finite metadata after the named unittest rows, so diagnostics
        # cannot interrupt their individual result lines.
        for case, stage, uid, gid, mode in cls.cluster_observations:
            print(f"PREUPGRADE_FIXTURE_CLUSTER:case={case}:stage={stage}:uid={uid}:gid={gid}:mode={mode:04o}",
                  flush=True)
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
            entry = next(e for e in manifest["entries"] if e["path"] == recovery.PGROOT)
            type(self).cluster_observations.append((self._testMethodName, "source",
                                                   entry["uid"], entry["gid"], entry["mode"]))
            self.storage.crypt(plain, cipher)
            self.check_stopped()
        self.storage.crypt(cipher, decrypted, decrypt=True)
        restored = stage / "restored"
        restored.mkdir(mode=0o700)
        with decrypted.open("rb") as stream:
            self.assertEqual(files.restore_archive(stream, self.storage.limits, str(restored),
                             self.storage.reserve, profile="preupgrade"), manifest)
        observed = (restored / recovery.PGROOT).stat(follow_symlinks=False)
        type(self).cluster_observations.append((self._testMethodName, "restored",
            observed.st_uid, observed.st_gid, stat.S_IMODE(observed.st_mode)))
        return restored, {"files": manifest, "database": copy.deepcopy(self.audit)}

    def assert_no_verifier(self):
        self.assertEqual(self.command(["ps", "--all", "--quiet", "--filter",
            "label=io.lagrange.preupgrade-project=" + self.project]).stdout.strip(), b"")
        self.check_stopped()
        self.assertFalse(any(p.name.startswith("preupgrade-verifier-")
                             for p in Path(self.storage.root).iterdir()))

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
            with self.assertRaisesRegex(recovery.runtime.RuntimeErrorCode,
                                        "^PREUPGRADE_RESTORE_AUDIT_MISMATCH$"):
                self.storage.verify_cluster(restored, expected)
            self.assert_no_verifier()

    def test_lost_create_response_recovers_only_owned_container_and_reports_failure(self):
        with self.storage.temporary(".preupgrade-") as stage:
            restored, expected = self.restore_files(stage)
            original = self.docker.call
            created = []
            def lost_response(args, timeout=15):
                result = original(args, timeout)
                if args[0] == "create":
                    self.assertRegex(result.strip(), "^[0-9a-f]{64}$")
                    created.append(result.strip())
                    raise recovery.runtime.RuntimeErrorCode("SIMULATED_LOST_CREATE_RESPONSE")
                return result
            with mock.patch.object(self.docker, "call", side_effect=lost_response), \
                    self.assertRaisesRegex(recovery.runtime.RuntimeErrorCode,
                                           "^SIMULATED_LOST_CREATE_RESPONSE$"):
                self.storage.verify_cluster(restored, expected)
            self.assertEqual(len(created), 1)
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
            with self.assertRaisesRegex(recovery.runtime.RuntimeErrorCode,
                                        "^PREUPGRADE_VERIFY_CONTROL$"):
                self.storage.verify_cluster(restored, expected)
            self.assert_no_verifier()


class PreupgradeFixturePreparationTests(unittest.TestCase):
    """Real private directory boundaries with controlled Docker observations."""

    def setUp(self):
        if os.geteuid() != 0 or sys.platform != "linux":
            self.fail("actual root Linux fixture required")
        self.directory = Path(tempfile.mkdtemp(prefix="lagrange-preupgrade-parent-", dir="/var/lib"))
        self.addCleanup(shutil.rmtree, self.directory)
        self.parent = self.directory / "fixture-volume"
        self.parent.mkdir(mode=0o711)
        self.mountpoint = self.parent / "_data"
        self.mountpoint.mkdir(mode=0o700)
        self.before = {"Name": "fixture-volume", "Driver": "local", "Scope": "local",
            "Options": None, "Labels": {"io.lagrange.preupgrade-fixture": "fixture-project"},
            "Mountpoint": str(self.mountpoint), "CreatedAt": "before-permission-change"}
        self.fixture = types.SimpleNamespace(docker=object(), volume="fixture-volume",
            project="fixture-project", mountpoint=str(self.mountpoint), check_stopped=mock.Mock())

    def test_parent_preparation_accepts_derived_creation_time_change_only(self):
        original, ancestor = self.parent.stat(), self.directory.stat()
        after = copy.deepcopy(self.before)
        after["CreatedAt"] = "after-permission-change"
        with mock.patch.object(recovery, "volume_info", side_effect=[self.before, after]), \
                mock.patch("builtins.print") as output:
            PreupgradeDockerTests.protect_fixture_volume_parent.__func__(self.fixture)
        final = self.parent.stat()
        self.assertEqual((final.st_dev, final.st_ino), (original.st_dev, original.st_ino))
        self.assertEqual((final.st_uid, final.st_gid, stat.S_IMODE(final.st_mode)), (0, 0, 0o700))
        self.assertEqual(self.directory.stat().st_mode, ancestor.st_mode)
        self.assertEqual(self.fixture.check_stopped.call_count, 2)
        output.assert_any_call("PREUPGRADE_FIXTURE_VOLUME_FIELDS:CreatedAt", flush=True)

    def test_parent_preparation_rejects_changed_stable_volume_fields(self):
        changes = {"Name": "different-volume", "Driver": "different-driver", "Scope": "global",
            "Options": {"device": "different-device"}, "Labels": {"owner": "different-owner"},
            "Mountpoint": "/different-mountpoint"}
        for field, value in changes.items():
            with self.subTest(field=field):
                self.parent.chmod(0o711)
                after = copy.deepcopy(self.before)
                after[field] = value
                with mock.patch.object(recovery, "volume_info", side_effect=[self.before, after]), \
                        mock.patch("builtins.print") as output, \
                        self.assertRaisesRegex(RuntimeError, "^PREUPGRADE_FIXTURE_VOLUME_CHANGED$"):
                    PreupgradeDockerTests.protect_fixture_volume_parent.__func__(self.fixture)
                output.assert_called_once_with("PREUPGRADE_FIXTURE_VOLUME_FIELDS:" + field, flush=True)

    def test_parent_preparation_rejects_replacement_during_volume_observation(self):
        observations = 0
        def observe(docker, volume):
            nonlocal observations
            self.assertIs(docker, self.fixture.docker)
            self.assertEqual(volume, self.fixture.volume)
            observations += 1
            if observations == 2:
                self.parent.rename(self.directory / "original-volume")
                self.parent.mkdir(mode=0o700)
                self.mountpoint.mkdir(mode=0o700)
            return copy.deepcopy(self.before)
        with mock.patch.object(recovery, "volume_info", side_effect=observe), \
                mock.patch("builtins.print"), \
                self.assertRaisesRegex(RuntimeError, "^PREUPGRADE_FIXTURE_VOLUME_CHANGED$"):
            PreupgradeDockerTests.protect_fixture_volume_parent.__func__(self.fixture)
        self.assertEqual(observations, 2)
        self.assertNotEqual(self.parent.stat().st_ino,
                            (self.directory / "original-volume").stat().st_ino)


if __name__ == "__main__":
    unittest.main(verbosity=2)
