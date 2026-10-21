#!/usr/bin/env python3
"""Actual PostgreSQL inspection tests in one owned, network-none full-schema fixture.

Reuse the first-session fixture's command/schema pattern, not its test suite.
No installed release, production database/config/secret, published port or pull.
"""
from __future__ import annotations

import datetime as dt
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest
import uuid
from dataclasses import replace

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


fixture = load("grant_pg_fixture", ROOT / "scripts/ops/owner-beta-daily-backup-docker-self-test.py")
pure = load("grant_pg_values", ROOT / "scripts/ops/test_market_stream_grant.py")
grant = pure.grant


class PrivateDatabase:
    admin_role = "lagrange"
    command = classmethod(fixture.DailyBackupDockerTests.command.__func__)
    sql = classmethod(fixture.DailyBackupDockerTests.sql.__func__)
    sqlstate = staticmethod(fixture.DailyBackupDockerTests.sqlstate)
    server = None

    @classmethod
    def start(cls):
        cls.directory = Path(tempfile.mkdtemp(prefix="lagrange-grant-inspect-", dir="/var/lib"))
        cls.name = "grant-inspect-" + uuid.uuid4().hex
        secret = cls.directory / "synthetic-password"
        secret.write_text(uuid.uuid4().hex)
        secret.chmod(0o400)
        os.chown(secret, 999, 999)
        result = cls.command(["run", "--detach", "--pull=never", "--name", cls.name,
            "--label", "io.lagrange.grant-inspect-fixture=" + cls.name,
            "--network=none", "--memory=384m", "--memory-swap=384m", "--cpus=0.5",
            "--pids-limit=128", "--env", "POSTGRES_USER=lagrange",
            "--env", "POSTGRES_DB=lagrange", "--env", "POSTGRES_PASSWORD_FILE=/run/test-secret",
            "--mount", "type=bind,src=" + str(secret) + ",dst=/run/test-secret,readonly",
            fixture.backup.POSTGRES, "-c", "listen_addresses=", "-c", "shared_buffers=32MB",
            "-c", "max_connections=10"])
        cls.server = result.stdout.decode("ascii").strip()
        if not fixture.backup.runtime.CONTAINER.fullmatch(cls.server):
            raise RuntimeError("GRANT_FIXTURE_IDENTITY")
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            # The image starts a temporary Unix-socket server during initdb.
            # Require the final postgres PID 1, then an actual identity query;
            # pg_isready alone can accept the temporary server before shutdown.
            probe = cls.command(["exec", "--user", "999:999", cls.server, "sh", "-c",
                '[ "$(cat /proc/1/comm)" = postgres ] || exit 1; '
                'exec psql -X -qAt -U lagrange -d lagrange -v ON_ERROR_STOP=1 '
                '-c "SELECT current_user || chr(58) || current_database()"'],
                timeout=5, allow_failure=True)
            if probe.returncode == 0 and probe.stdout == b"lagrange:lagrange\n":
                break
            time.sleep(0.25)
        else:
            raise RuntimeError("GRANT_FIXTURE_START_TIMEOUT")
        roles = ("migration_owner", "app", "worker", "admin", "research_writer", "audit_writer", "daily_input_admitter")
        cls.sql("".join("CREATE ROLE " + role + " LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE "
                        "NOREPLICATION NOBYPASSRLS;\n" for role in roles) +
                "GRANT CONNECT ON DATABASE lagrange TO " + ",".join(roles) + ";\n"
                "GRANT USAGE,CREATE ON SCHEMA public TO migration_owner;\n"
                "GRANT USAGE ON SCHEMA public TO " + ",".join(roles[1:]) + ";\n", stage="roles")
        migrations = sorted((ROOT / "migrations").glob("*.up.sql"))
        if len(migrations) != 62:
            raise RuntimeError("GRANT_FIXTURE_MIGRATION_INVENTORY")
        for path in migrations:
            script = path.read_text()
            if not script.startswith("-- no-transaction\n"):
                script = "BEGIN;\n" + script + "\nCOMMIT;"
            cls.sql(script, role="migration_owner", stage="migration")
        cls.migration_hash = hashlib.sha256(b"".join(path.read_bytes() for path in migrations)).hexdigest()
        cls.sql("INSERT INTO users(id,issuer,subject,email) VALUES('" + pure.OWNER +
                "','grant-fixture','grant-fixture','grant-fixture@example.invalid');"
                "INSERT INTO roles(id) VALUES('owner') ON CONFLICT DO NOTHING;"
                "INSERT INTO user_roles(user_id,role_id) VALUES('" + pure.OWNER + "','owner');"
                "INSERT INTO data_entitlements(id,contract_document_sha256,contract_reference,status,"
                "covered_datasets,covered_uses,effective_from,effective_until,managed_by) VALUES('" +
                pure.ENTITLEMENT + "','" + "a" * 64 + "','fixture grant','ACTIVE','[]','[]',"
                "'2000-01-01','2099-12-31','" + pure.OWNER + "');", stage="seed")
        print("GRANT_INSPECT_FIXTURE: migrations=62 sha256=" + cls.migration_hash, flush=True)
        print("GRANT_INSPECT_FIXTURE: container=" + cls.server + " name=" + cls.name, flush=True)

    @classmethod
    def cleanup(cls):
        if not hasattr(cls, "directory"):
            return
        # Resolve only our unpredictable exact name, including ambiguous run outcomes.
        found = cls.command(["ps", "-aq", "--no-trunc", "--filter", "name=^/" + cls.name + "$"],
                            timeout=10).stdout.decode().splitlines()
        if len(found) > 1:
            raise RuntimeError("GRANT_FIXTURE_CLEANUP_IDENTITY")
        for identity in found:
            if not fixture.backup.runtime.CONTAINER.fullmatch(identity):
                raise RuntimeError("GRANT_FIXTURE_CLEANUP_IDENTITY")
            binding = cls.command(["inspect", "--format",
                '{{.Name}}|{{index .Config.Labels "io.lagrange.grant-inspect-fixture"}}', identity]).stdout
            if binding != ("/" + cls.name + "|" + cls.name + "\n").encode():
                raise RuntimeError("GRANT_FIXTURE_CLEANUP_BINDING")
            volumes = json.loads(cls.command(["inspect", "--format", '{{json .Mounts}}', identity]).stdout)
            owned_volumes = [m["Name"] for m in volumes if m["Type"] == "volume"]
            cls.command(["rm", "--force", "--volumes", identity], timeout=15)
            for volume in owned_volumes:
                if cls.command(["volume", "ls", "-q", "--filter", "name=^" + volume + "$"], timeout=10).stdout.strip():
                    raise RuntimeError("GRANT_FIXTURE_VOLUME_REMAINS")
        if cls.command(["ps", "-aq", "--no-trunc", "--filter", "name=^/" + cls.name + "$"], timeout=10).stdout.strip():
            raise RuntimeError("GRANT_FIXTURE_CONTAINER_REMAINS")
        shutil.rmtree(cls.directory)
        if cls.directory.exists():
            raise RuntimeError("GRANT_FIXTURE_DIRECTORY_REMAINS")
        print("GRANT_INSPECT_CLEANUP: PASS container_absent volumes_absent directory_absent name=" + cls.name, flush=True)


class GrantInspectionPostgresTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if os.geteuid() != 0:
            raise RuntimeError("GRANT_INSPECT_ROOT_FIXTURE_REQUIRED")
        cls.addClassCleanup(PrivateDatabase.cleanup)
        PrivateDatabase.start()
        cls.database = PrivateDatabase
        cls.sql_bytes = (ROOT / "scripts/ops/lib/market-stream-grant-inspect.sql").read_bytes()
        cls.today = dt.date.fromisoformat(cls.database.sql(
            "SELECT (transaction_timestamp() AT TIME ZONE 'Asia/Seoul')::date;").decode().strip())

    def setUp(self):
        self.payload = pure.approval_fixture()
        self.payload.update(entitlement_reference="fixture grant",
            effective_from=(self.today - dt.timedelta(days=1)).isoformat(),
            effective_until=(self.today + dt.timedelta(days=1)).isoformat())
        self.database.sql("TRUNCATE public.owner_market_stream_grants CASCADE;"
            "INSERT INTO user_roles(user_id,role_id) VALUES('" + pure.OWNER + "','owner') ON CONFLICT DO NOTHING;"
            "UPDATE data_entitlements SET status='ACTIVE',contract_reference='fixture grant',"
            "contract_document_sha256='" + "a" * 64 + "',effective_from='2000-01-01',effective_until='2099-12-31';")

    def seed(self, *, state="ACTIVE"):
        columns = [key for key in self.payload if key not in {"scope", "schema_version"}]
        names = ["id" if key == "grant_id" else key for key in columns]
        values = ["'" + self.payload[key].replace("'", "''") + "'" for key in columns]
        self.database.sql("INSERT INTO public.owner_market_stream_grants(" + ",".join(names) +
            ",state,revoked_at) VALUES(" + ",".join(values) + ",'" + state + "'," +
            ("NULL" if state == "ACTIVE" else "clock_timestamp()") + ");", role="migration_owner")

    def snapshot(self):
        tables = ["owner_market_stream_grants", "user_roles", "data_entitlements",
                  "owner_market_stream_leases", "owner_market_stream_lease_items",
                  "owner_market_stream_producers", "owner_market_stream_subscriptions", "owner_market_stream_cache"]
        return self.database.sql("SELECT " + " || ':' || ".join(
            "(SELECT md5(coalesce(string_agg(to_jsonb(t)::text,',' ORDER BY to_jsonb(t)::text),'')) FROM public." +
            table + " t)" for table in tables) + ";")

    def inspect(self, payload=None, *, role="migration_owner", sql=None, mode="inspect", grant_id=pure.GRANT):
        before = self.snapshot()
        result = self.database.command(["exec", "--interactive", "--user", "999:999",
            "--env", "PGOPTIONS=-c default_transaction_read_only=on", self.database.server,
            "psql", "-X", "-qAt", "-U", role, "-d", "lagrange", "-v", "ON_ERROR_STOP=1",
            "-v", "VERBOSITY=sqlstate", "-v", "grant_payload=" + json.dumps(self.payload if payload is None else payload),
            "-v", "grant_mode=" + mode, "-v", "grant_id=" + grant_id],
            self.sql_bytes if sql is None else sql, timeout=40, allow_failure=True)
        self.assertEqual(self.snapshot(), before, "inspection changed fixture rows")
        return result

    def expect(self, status, **kwargs):
        result = self.inspect(**kwargs)
        self.assertEqual(result.returncode, 0, self.database.sqlstate(result))
        self.assertEqual(result.stderr, b"")
        self.assertEqual(result.stdout, (status + "\n").encode())

    def test_actual_valid_and_no_row_mutation(self):
        self.seed(); self.expect("INSPECT_VALID")

    def test_actual_missing_is_only_successful_absence(self):
        self.expect("INSPECT_MISSING")
        broken = self.sql_bytes.replace(b"public.owner_market_stream_grants AS g", b"public.absent_fixture_table AS g")
        result = self.inspect(sql=broken)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn(b"INSPECT_MISSING", result.stdout)

    def test_actual_revoked(self):
        self.seed(state="REVOKED"); self.expect("INSPECT_REVOKED")

    def test_actual_expired_and_not_yet_valid(self):
        self.payload.update(effective_from=(self.today - dt.timedelta(days=2)).isoformat(),
                            effective_until=(self.today - dt.timedelta(days=1)).isoformat())
        self.seed(); self.expect("INSPECT_EXPIRED")
        self.database.sql("TRUNCATE public.owner_market_stream_grants CASCADE;")
        self.payload.update(effective_from=(self.today + dt.timedelta(days=1)).isoformat(),
                            effective_until=(self.today + dt.timedelta(days=2)).isoformat())
        self.seed(); self.expect("INSPECT_NOT_YET_VALID")

    def test_actual_effective_date_boundaries_are_inclusive(self):
        self.payload.update(effective_from=self.today.isoformat(), effective_until=self.today.isoformat())
        self.seed(); self.expect("INSPECT_VALID")

    def test_actual_every_immutable_field_and_json_type_mismatch(self):
        self.seed()
        for key in self.payload.keys() - {"scope", "schema_version", "grant_id"}:
            for value in ("different", None, 7):
                with self.subTest(field=key, value=value):
                    self.expect("INSPECT_MISMATCH", payload={**self.payload, key:value})
        self.expect("DB_OUTCOME_UNKNOWN", payload={**self.payload, "grant_id": pure.REVISION})
        self.expect("INSPECT_MISMATCH", payload={**self.payload, "extra":"forbidden"})

    def test_actual_owner_and_entitlement_eligibility(self):
        self.seed()
        self.database.sql("DELETE FROM user_roles WHERE user_id='" + pure.OWNER + "';")
        self.expect("INSPECT_MISMATCH")
        self.database.sql("INSERT INTO user_roles(user_id,role_id) VALUES('" + pure.OWNER + "','owner');")
        for update, restore in (
            ("status='REVOKED'", "status='ACTIVE'"),
            ("contract_reference='different'", "contract_reference='fixture grant'"),
            ("contract_document_sha256='" + "b" * 64 + "'", "contract_document_sha256='" + "a" * 64 + "'"),
            ("effective_until='2001-01-01'", "effective_until='2099-12-31'"),
            ("effective_from='2098-01-01'", "effective_from='2000-01-01'"),
        ):
            with self.subTest(update=update):
                self.database.sql("UPDATE data_entitlements SET " + update + ";")
                self.expect("INSPECT_MISMATCH")
                self.database.sql("UPDATE data_entitlements SET " + restore + ";")

    def test_actual_wrong_role_and_mode_fail_closed(self):
        self.seed()
        self.expect("DB_OUTCOME_UNKNOWN", role="lagrange")
        self.expect("DB_OUTCOME_UNKNOWN", mode="install")
        self.expect("DB_OUTCOME_UNKNOWN", sql=b"SET ROLE migration_owner;\n" + self.sql_bytes, role="lagrange")
        for role in ("app", "worker"):
            with self.subTest(role=role):
                result = self.inspect(role=role)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn(b"INSPECT_MISSING", result.stdout)

    def test_actual_read_only_blocks_mutation(self):
        self.seed()
        # Adversarial injection into the same read-only transaction, fixture only.
        for statement in (b"UPDATE public.data_entitlements SET status='REVOKED';\n",
                          b"DELETE FROM public.user_roles;\n"):
            result = self.inspect(sql=self.sql_bytes.replace(b"WITH expected", statement + b"WITH expected"))
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(self.database.sqlstate(result), "25006")
            self.assertNotIn(b"INSPECT_VALID", result.stdout)

    def test_actual_fixture_is_networkless_and_real_migration_owner(self):
        value = json.loads(self.database.command(["inspect", "--format", '{{json .HostConfig}}', self.database.server]).stdout)
        self.assertEqual(value["NetworkMode"], "none")
        self.assertFalse(value["PortBindings"])
        self.assertEqual(value["Memory"], 384 * 1024 * 1024)
        self.assertEqual(value["NanoCpus"], 500000000)
        self.assertEqual(self.database.sql("SELECT current_user||':'||session_user||':'||"
            "(SELECT rolsuper::text FROM pg_roles WHERE rolname=current_user)||':'||"
            "current_setting('listen_addresses');", role="migration_owner").strip(),
            b"migration_owner:migration_owner:false:")

    def test_packaged_sql_trusted_path_rejects_symlink_and_writable_file(self):
        relative = "scripts/ops/lib/market-stream-grant-inspect.sql"
        archive = subprocess.run(["git", "-c", "safe.directory=" + str(ROOT), "-C", str(ROOT),
            "archive", "HEAD", relative], capture_output=True, timeout=10, check=True).stdout
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            member = tar.getmember(relative)
            self.assertTrue(member.isfile())
            self.assertEqual(tar.extractfile(member).read(), self.sql_bytes)
        release = self.database.directory / "release"
        release.mkdir(mode=0o755)
        target = release / "market-stream-grant-inspect.sql"
        target.write_bytes(self.sql_bytes); target.chmod(0o644)
        context = replace(pure.context_fixture(), sql_file=release / "market-stream-grant.sql")
        self.assertEqual(grant._read_trusted_sql(context, "inspect"), self.sql_bytes)
        target.chmod(0o666)
        with self.assertRaises(grant.GrantFailure): grant._read_trusted_sql(context, "inspect")
        target.chmod(0o644); target.rename(release / "original.sql")
        target.symlink_to(release / "original.sql")
        with self.assertRaises(grant.GrantFailure): grant._read_trusted_sql(context, "inspect")


if __name__ == "__main__":
    unittest.main(verbosity=2)
