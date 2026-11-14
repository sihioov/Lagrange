#!/usr/bin/env python3
"""Pure resource decisions only. These mocks never mint hosted authority."""
import copy
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import pickle
import sys
import tempfile
import types
import unittest
from unittest import mock

sys.dont_write_bytecode = True
PATH = Path(__file__).with_name("lib") / "owner-beta-paper-backup.py"
SPEC = importlib.util.spec_from_file_location("owner_beta_paper_backup", PATH)
backup = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = backup
SPEC.loader.exec_module(backup)


class PaperCaptureTests(unittest.TestCase):
    def test_dataset_capture_uses_only_existing_exporter_and_exact_identity(self):
        owner = object.__new__(backup._HeldPaperSnapshot)
        owner.revalidate = mock.Mock()
        owner.exporter = mock.Mock()
        identity = "11111111-1111-4111-8111-111111111111"
        row = dict(id=identity, dataset_id="demo", version="1", status="CURATED",
                   manifest_sha256="a"*64, storage_path="/data/curated/demo/1")
        def exchange(sql, destination, *, maximum):
            self.assertIn(("WHERE id='" + identity + "'::uuid").encode(), sql)
            self.assertEqual(maximum, 2*1024*1024)
            destination.write(backup.json.dumps(row).encode() + b"\n")
        owner.exporter.exchange.side_effect = exchange
        self.assertEqual(owner._capture_dataset_rows((identity,)), [row])
        owner.exporter.exchange.assert_called_once()
        row["id"] = "22222222-2222-4222-8222-222222222222"
        with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_DATASET_ROW"):
            owner._capture_dataset_rows((identity,))

    def test_empty_dataset_set_never_opens_or_queries_another_client(self):
        owner = object.__new__(backup._HeldPaperSnapshot)
        owner.revalidate = mock.Mock()
        owner.exporter = mock.Mock()
        self.assertEqual(owner._capture_dataset_rows(()), [])
        owner.exporter.exchange.assert_not_called()

    def test_archive_descriptor_binds_raw_spool_not_audit_document_and_never_snapshot_token(self):
        import contextlib
        owner = object.__new__(backup._HeldPaperSnapshot)
        owner.revalidate = mock.Mock()
        canonical = lambda value: backup.json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
        digest = lambda data: "sha256:" + backup.hashlib.sha256(data).hexdigest()
        cap = canonical({"migration_0068_sha256": "sha256:"+"6"*64})
        verified = types.SimpleNamespace(revalidate=mock.Mock(), _fds=[8], capability_sha256=digest(cap))
        owner.context = types.SimpleNamespace(verified=verified)
        owner.operation = types.SimpleNamespace(_intent=dict(installation_id="source", generation="generation",
            account_id="account", release_manifest_sha256="sha256:"+"1"*64))
        consumer = types.SimpleNamespace(floor=2, actions=[types.SimpleNamespace(_artifact_bytes=b"drain")])
        owner.drained = types.SimpleNamespace(_attempt=types.SimpleNamespace(consumer=consumer))
        owner.sources = [types.SimpleNamespace(sha256="sha256:"+"2"*64)]
        owner.description = {"groups": [dict(name="daily", sha256="sha256:"+"3"*64, row_count=7, byte_length=90)]}
        owner.spools = {name: (None,None,5,"sha256:"+char*64) for name,char in
                        (("postgres.dump","4"),("audit.stream","5"),("files.bundle","7"))}
        owner._snapshot = "NEVER_PERSIST_LIVE_TOKEN"
        files = types.SimpleNamespace(class_records=mock.Mock(return_value=[]))
        owner.selection = types.SimpleNamespace(_backup_module=lambda: types.SimpleNamespace(files=files))
        daily = types.SimpleNamespace(read_file=mock.Mock(return_value=cap))
        release = types.SimpleNamespace(_daily=lambda:daily, MAX_PROCESS=65536,
                                        _json=lambda data,limit:backup.json.loads(data))
        owner.ops = types.SimpleNamespace(_canonical=canonical, _digest=digest, release=release)
        saved = {}
        @contextlib.contextmanager
        def spool(name):
            stream = io.BytesIO()
            yield stream
            saved[name] = stream.getvalue()
        owner._new_spool = spool
        owner._seal_spool = mock.Mock()
        owner._describe_archive(mock.Mock(), dict(group="source_files",sha256="sha256:"+"8"*64,
                                                  entries=1,bytes=5), object())
        audit = backup.json.loads(saved["audit.json"])
        archive = backup.json.loads(saved["archive.json"])
        self.assertEqual(audit["groups"][0]["entries"], 7)
        self.assertEqual(audit["groups"][0]["bytes"], 90)
        binding = dict(domain="lagrange-paper-snapshot-binding-v1",installation_id="source",
            generation="generation",account_id="account",control_epoch=2,
            database_dump_sha256="sha256:"+"4"*64,audit_stream_sha256="sha256:"+"5"*64,
            files_sha256="sha256:"+"7"*64)
        self.assertEqual(archive["database_snapshot_sha256"], digest(canonical(binding)))
        self.assertEqual(archive["audit_sha256"], digest(saved["audit.json"]))
        self.assertNotEqual(archive["audit_sha256"], binding["audit_stream_sha256"])
        self.assertNotIn(owner._snapshot.encode(), b"".join(saved.values()))
        owner._seal_spool.assert_has_calls([mock.call("audit.json",mock.ANY), mock.call("archive.json",mock.ANY)])

    def test_archive_package_cannot_be_constructed_from_attestations(self):
        for value in ({"verified": True}, object(), None):
            with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_PRIVATE"):
                backup._ArchivePackage(value, object(), {})

    def test_archive_sql_quotes_source_identifiers_and_literals(self):
        self.assertEqual(backup._sql_name('a"b'), '"a""b"')
        self.assertEqual(backup._sql_literal("a'b\\c"), "'a''b\\c'")
        for value in ("", "x\0y", "x\ny", 1):
            with self.assertRaises(backup.PaperBackupError):
                backup._sql_name(value)

    def test_archive_database_acl_keeps_grantor_and_grant_option(self):
        sql = backup._database_acl_sql("db", ["=c/owner", "app=C*T/owner"], "owner")
        self.assertIn('GRANT CONNECT ON DATABASE "db" TO PUBLIC', sql)
        self.assertIn('GRANT CREATE ON DATABASE "db" TO "app" WITH GRANT OPTION', sql)
        self.assertIn('GRANTED BY "owner"', sql)
        for value in (["app=X/owner"], ["app=C/"], ["app=C/owner/extra"]):
            with self.assertRaises(backup.PaperBackupError):
                backup._database_acl_sql("db", value, "owner")

    def test_archive_guest_and_package_are_not_serializable(self):
        for cls in (backup._ArchivePackage, backup._PaperRestoreGuest):
            value = object.__new__(cls)
            for operation in (copy.copy, copy.deepcopy, pickle.dumps):
                with self.assertRaises(backup.PaperBackupError):
                    operation(value)

    def test_archive_package_unknown_close_visits_both_owners_and_retains_originals(self):
        value = object.__new__(backup._ArchivePackage)
        value.closed = False
        value.owner = types.SimpleNamespace(_poisoned=False)
        value.call = mock.Mock()
        value.call.close.side_effect = backup.PaperBackupError("PAPER_BACKUP_NATIVE_INCOMPLETE")
        value.guest = mock.Mock()
        value.process = None
        value.originals = [mock.Mock()]
        with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_CLEANUP_UNKNOWN"):
            value.close()
        value.guest.close.assert_called_once()
        value.originals[0].close.assert_not_called()
        self.assertFalse(value.closed)
        self.assertTrue(value.owner._poisoned)

    def test_archive_capture_preserves_primary_and_poison_on_failed_disposal(self):
        owner = object.__new__(backup._HeldPaperSnapshot)
        owner._poisoned = False
        package = mock.Mock()
        package.invoke.side_effect = backup.PaperBackupError("PAPER_BACKUP_NATIVE_INCOMPLETE")
        package.close.side_effect = backup.PaperBackupError("PAPER_BACKUP_CLEANUP_UNKNOWN")
        with mock.patch.object(backup, "_ArchivePackage", return_value=package):
            with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_NATIVE_INCOMPLETE"):
                owner._archive_capture(object(), {})
        package.prepare.assert_called_once()
        package.invoke.assert_called_once()
        package.close.assert_called_once()
        self.assertTrue(owner._poisoned)

    def test_archive_original_rejects_named_swap_even_when_bytes_match(self):
        original = object.__new__(backup._ArchiveOriginal)
        original.fd, original.name = 8, "files.bundle"
        original.tree = types.SimpleNamespace(fd=7, revalidate=mock.Mock())
        original.identity, original.size, original.sha256 = "old", 9, "hash"
        original.ops = types.SimpleNamespace(_identity=lambda value: value,
                                             _hash_fd=lambda fd: (9, "hash"))
        with mock.patch.object(backup.os, "fstat", return_value="old"), \
                mock.patch.object(backup.os, "stat", return_value="replacement"):
            with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_ORIGINAL_CHANGED"):
                original.revalidate(None)

    def test_archive_guest_never_creates_after_catalog_failure(self):
        guest = object.__new__(backup._PaperRestoreGuest)
        guest.owner = mock.Mock()
        guest.package = mock.Mock()
        guest.create_sent, guest.cid, guest.absent = False, None, False
        with mock.patch.object(backup,"_PackageAuditView"), mock.patch.object(backup, "_catalog_restore_sql", side_effect=backup.PaperBackupError("PAPER_BACKUP_CATALOG")):
            with self.assertRaises(backup.PaperBackupError):
                guest.start()
        guest.owner._docker.assert_not_called()

    def test_archive_guest_lost_create_retains_identity_without_guessing_removal(self):
        guest = object.__new__(backup._PaperRestoreGuest)
        guest.absent, guest.cid, guest.create_sent = False, None, True
        guest.owner = mock.Mock()
        with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_CLONE_UNKNOWN"):
            guest.close()
        guest.owner._cleanup_docker.assert_not_called()
        self.assertFalse(guest.absent)

    def test_archive_guest_not_created_can_release_only_its_empty_custody(self):
        guest = object.__new__(backup._PaperRestoreGuest)
        guest.absent, guest.cid, guest.create_sent, guest.directories = False, None, False, []
        guest.owner = mock.Mock()
        guest.close()
        self.assertTrue(guest.absent)
        guest.owner._cleanup_docker.assert_not_called()

    def test_archive_guest_checks_exact_generation_network_mount_and_policy(self):
        guest = object.__new__(backup._PaperRestoreGuest)
        guest.absent, guest.cid, guest.created, guest.image = False, "cid", "created", "image"
        guest.name, guest.nonce, guest.admin = "owned", "nonce", "owner"
        guest.directories, guest.started_at, guest.mounts = [], "started", (("src", "dst", True),)
        guest.package = types.SimpleNamespace(database={"database": "captured"})
        guest.ops = types.SimpleNamespace(_validate_mount_records=mock.Mock())
        value = dict(Id="cid", Created="created", Image="image", Name="/owned", RestartCount=0,
            Config=dict(Labels={"lagrange.paper.archive": "nonce"}, User="999:999",
                        Entrypoint=["/bin/bash"], Cmd=["/verify/entry.sh", "owner", "captured"]),
            HostConfig=dict(NetworkMode="none", ReadonlyRootfs=True, Privileged=False, CapDrop=["ALL"],
                RestartPolicy={"Name": "no"}, AutoRemove=False, Memory=2147483648, NanoCpus=1000000000,
                LogConfig={"Type": "none"}, SecurityOpt=["no-new-privileges:true"], PidMode="", IpcMode="private",
                Tmpfs={backup._IMAGE_VOLUME: backup._GUEST_VOLUME_COVER}),
            State=dict(StartedAt="started", OOMKilled=False), Mounts=["actual"],
            NetworkSettings={"Networks": {"none": {"IPAddress": ""}}})
        guest.owner = types.SimpleNamespace(context=types.SimpleNamespace(docker=mock.Mock()))
        guest.owner.context.docker.object.return_value = value
        guest.check()
        guest.ops._validate_mount_records.assert_called_once_with(["actual"], guest.mounts)
        for group, key, changed in ((None, "Id", "foreign"), (None, "RestartCount", 1),
                ("State", "StartedAt", "replacement"), ("HostConfig", "NetworkMode", "host"),
                ("HostConfig", "Privileged", True), ("Config", "User", "0:0")):
            altered = copy.deepcopy(value)
            (altered if group is None else altered[group])[key] = changed
            guest.owner.context.docker.object.return_value = altered
            with self.subTest(group=group, key=key), self.assertRaisesRegex(
                    backup.PaperBackupError, "PAPER_BACKUP_CLONE_CHANGED"):
                guest.check()

    def test_archive_cipher_rejects_expired_original_budget_before_spawn(self):
        package = object.__new__(backup._ArchivePackage)
        with mock.patch.object(backup._ArchivePackage, "_reserve", side_effect=backup.PaperBackupError("PAPER_BACKUP_TIMEOUT")), \
                mock.patch.object(backup.subprocess, "Popen") as spawn:
            with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_TIMEOUT"):
                package._command_file([], "/never-created")
        spawn.assert_not_called()

    def catalog(self):
        role = dict(rolname="owner", rolsuper=True, rolinherit=True, rolcreaterole=True,
            rolcreatedb=True, rolcanlogin=True, rolreplication=True, rolbypassrls=True,
            rolconnlimit=-1, rolconfig=None, rolvaliduntil=None)
        migration = dict(role, rolname="migration_owner", rolsuper=False,
                         rolcreaterole=False, rolcreatedb=False, rolreplication=False, rolbypassrls=False)
        database = dict(owner="owner", acl=["=c/owner", "owner=CTc/owner"], encoding="UTF8",
                        collate="C", ctype="C", allow_connections=True, connection_limit=-1)
        rows = [["row", "schema_security", "pg_roles", r["rolname"], r] for r in (role, migration)]
        rows.append(["row", "schema_security", "pg_database", "database", database])
        return rows

    def test_archive_catalog_preserves_actual_login_attributes_database_and_acl(self):
        import json
        rows = self.catalog()
        rows.append(["row", "schema_security", "pg_auth_members", "x", dict(
            role="owner", member="migration_owner", grantor="owner",
            admin_option=False, inherit_option=False, set_option=True)])
        owner = types.SimpleNamespace(_check_spools=mock.Mock(),
            audit=io.BytesIO(b"".join(json.dumps(r).encode()+b"\n" for r in rows)),
            audit_module=types.SimpleNamespace(decode=mock.Mock(),
                parse=backup._local("paper_catalog_decimal_audit","owner-beta-paper-audit.py").parse,
                canonical=backup._local("paper_catalog_decimal_audit","owner-beta-paper-audit.py").canonical),
            context=types.SimpleNamespace(database={"database": "captured_database"}))
        admin, sql = backup._catalog_restore_sql(owner)
        self.assertEqual(admin, "owner")
        text = sql.decode()
        self.assertIn('ALTER ROLE "migration_owner" WITH NOSUPERUSER INHERIT NOCREATEROLE NOCREATEDB LOGIN', text)
        self.assertIn('CREATE DATABASE "captured_database" WITH TEMPLATE template0 OWNER "owner"', text)
        self.assertIn('WITH ADMIN FALSE, INHERIT FALSE, SET TRUE GRANTED BY "owner"', text)
        self.assertNotIn("PASSWORD", text)
        owner.audit_module.decode.assert_called_once()
        self.assertEqual(owner._check_spools.call_count, 2)

    def test_archive_catalog_rejects_missing_direct_login_principal(self):
        import json
        rows = self.catalog()
        rows[1][4]["rolcanlogin"] = False
        owner = types.SimpleNamespace(_check_spools=mock.Mock(),
            audit=io.BytesIO(b"".join(json.dumps(r).encode()+b"\n" for r in rows)),
            audit_module=types.SimpleNamespace(decode=mock.Mock(),
                parse=backup._local("paper_catalog_decimal_audit","owner-beta-paper-audit.py").parse,
                canonical=backup._local("paper_catalog_decimal_audit","owner-beta-paper-audit.py").canonical))
        with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_CATALOG"):
            backup._catalog_restore_sql(owner)

    def test_public_factory_rejects_dicts_and_private_constructor_rejects_token(self):
        with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_PRIVATE"):
            backup._HeldPaperSnapshot(object(), {}, {}, 10)
        with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_CONTEXT"):
            backup.open_snapshot({}, {}, 10)

    def test_noncopyable_nonpickleable(self):
        owner = object.__new__(backup._HeldPaperSnapshot)
        for operation in (copy.copy, copy.deepcopy, pickle.dumps):
            with self.subTest(operation=operation.__name__), self.assertRaises(backup.PaperBackupError):
                operation(owner)

    def test_pgpass_escapes_original_bytes_without_argument_or_environment(self):
        self.assertEqual(backup._pgpass(b"a:b\\c", "lagrange"),
                         b"postgres:5432:lagrange:migration_owner:a\\:b\\\\c\n")
        for bad in (b"", b"a\n", b"a\r", b"a\0"):
            with self.assertRaises(backup.PaperBackupError):
                backup._pgpass(bad, "lagrange")
        with self.assertRaises(backup.PaperBackupError):
            backup._pgpass(b"secret", "foreign:database")

    def owner(self):
        owner = object.__new__(backup._HeldPaperSnapshot)
        owner.context = types.SimpleNamespace(database_generation=("server",),
            database={"database": "lagrange"}, docker=mock.Mock())
        owner.context.docker.object.return_value = {"NetworkSettings": {"Networks": {"n": {"IPAddress": "10.0.0.1"}}}}
        owner.ops = types.SimpleNamespace(_tcp_endpoints=mock.Mock())
        row = {"pid": 10, "start_us": 20, "role": "migration_owner", "addr": "10.0.0.2", "port": 30100}
        client = types.SimpleNamespace(backend=row, absent=False)
        owner.clients = [client]
        return owner, client, row

    def test_actual_backend_set_cannot_be_waived_by_role_or_application(self):
        owner, client, row = self.owner()
        owner._match_clients([dict(row)])
        for rows in ([], [dict(row, start_us=21)], [dict(row), dict(row, pid=11)],
                     [dict(row, role="app")], [dict(row, port=30101)]):
            with self.subTest(rows=len(rows)), self.assertRaises(backup.PaperBackupError):
                owner._match_clients(rows)

    def test_archive_failure_poison_is_permanent(self):
        owner = object.__new__(backup._HeldPaperSnapshot)
        owner._poisoned = False
        owner._custody = types.SimpleNamespace(revalidate=mock.Mock(side_effect=RuntimeError("expired")))
        with self.assertRaises(RuntimeError):
            owner.revalidate()
        self.assertTrue(owner._poisoned)

    def test_selection_tree_can_be_taken_only_once(self):
        owner = object.__new__(backup._HeldPaperSnapshot)
        owner._selection_taken = False
        owner.selection_tree = object()
        with mock.patch.object(backup._HeldPaperSnapshot, "revalidate"):
            self.assertIs(owner._take_selection_tree(), owner.selection_tree)
            with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_SELECTION_REPLAY"):
                owner._take_selection_tree()

    def test_incomplete_capture_consumes_real_methods_but_never_publishes(self):
        owner = object.__new__(backup._HeldPaperSnapshot)
        owner.context, owner.drained = object(), object()
        selected = mock.Mock()
        selected.__enter__ = mock.Mock(return_value=selected)
        selected.__exit__ = mock.Mock(return_value=False)
        selected.snapshot.write.side_effect = lambda stream: stream.write(b"component-bundle")
        pending = mock.Mock()
        pending.__enter__ = mock.Mock(return_value=pending)
        pending.__exit__ = mock.Mock(return_value=False)
        refs = types.SimpleNamespace(prepare_references=mock.Mock(return_value=pending),
            dataset_requirements=mock.Mock(return_value=()), encode_dataset_rows=mock.Mock(return_value=b""),
            describe=mock.Mock(return_value={"group": "source_files"}))
        owner.selection = types.SimpleNamespace(open_selected_paper_files=mock.Mock(return_value=selected))
        owner.audit = io.BytesIO(b"component-stream")
        with tempfile.TemporaryDirectory() as root, \
                mock.patch.object(backup._HeldPaperSnapshot, "revalidate"), \
                mock.patch.object(backup._HeldPaperSnapshot, "_dump") as dump, \
                mock.patch.object(backup._HeldPaperSnapshot, "_archive_capture") as archive, \
                mock.patch.object(backup._HeldPaperSnapshot, "_capture_dataset_rows", return_value=[]), \
                mock.patch.object(backup._HeldPaperSnapshot, "_describe_archive"), \
                mock.patch.object(backup, "_local", return_value=refs):
            owner.root = Path(root)
            owner._new_spool = lambda name: open(owner.root / name, "x+b")
            owner._seal_spool = mock.Mock()
            with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_REFERENCE_CLOSURE_INCOMPLETE"):
                owner.capture(object())
            dump.assert_called_once()
            refs.prepare_references.assert_called_once_with(selected.snapshot, owner.audit)
            pending.visit_archive_members.assert_called_once()
            pending.visit_database_references.assert_called_once()
            archive.assert_called_once()
            self.assertEqual(sorted(p.name for p in owner.root.iterdir()), ["dataset-rows.jsonl", "files.bundle"])
            self.assertIsNone(owner.selected)
        selected.__exit__.assert_called_once()

    def test_reservation_cannot_dispose_before_owned_clients_and_stop_locks_close(self):
        space = object.__new__(backup._CaptureSpace)
        space.closed = False
        space.owner = types.SimpleNamespace(_closed=False)
        with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_CLEANUP_UNKNOWN"):
            space.close()
        space.owner._closed = True
        space.context = object()
        space.ops = types.SimpleNamespace(_STOP_ATTEMPTS={space.context: {object(): types.SimpleNamespace(closed=False)}})
        with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_CLEANUP_UNKNOWN"):
            space.close()
        self.assertFalse(space.closed)

    def test_run_cleanup_retains_borrowed_originals_on_unknown_client(self):
        live = mock.Mock()
        run = backup._CaptureRun(live)
        owner = types.SimpleNamespace(close=mock.Mock(side_effect=backup.PaperBackupError("PAPER_BACKUP_CLEANUP_UNKNOWN")))
        run.space = types.SimpleNamespace(owner=owner, close=mock.Mock())
        run.drained = mock.Mock()
        run.operation, run.action, run.manager = mock.Mock(), mock.Mock(), mock.Mock()
        try:
            with self.assertRaises(backup.PaperBackupError):
                run.close()
            self.assertIn(run, backup._RUNS)
            for held in (run.drained, run.operation, run.action, live):
                held.close.assert_not_called()
            run.manager.__exit__ = mock.Mock()
            owner.close.side_effect = None
            run.close()
            self.assertNotIn(run, backup._RUNS)
            run.space.close.assert_called_once()
        finally:
            backup._RUNS.discard(run)

    def test_locked_prefix_preserves_legacy_only_on_actual_empty_probe(self):
        empty = object()
        for presence, configured, expected in ((empty, False, None), (object(), False, "PAPER_BACKUP_CONFIGURATION_UNKNOWN"),
                                               (object(), True, "PAPER_BACKUP_RESERVATION_REQUIRED")):
            with self.subTest(configured=configured):
                inventory = types.SimpleNamespace(_presence=presence, _configured=configured,
                    revalidate=mock.Mock(), _pages=[(None,b"page")], _local=types.SimpleNamespace(operations=[]))
                live = types.SimpleNamespace(probe_history=lambda: __import__("contextlib").nullcontext(inventory), close=mock.Mock())
                ops = types.SimpleNamespace(_HISTORY_EMPTY=empty, MAX_PAGE=4096,
                    _json=lambda *args: {"account_id": "account", "control_epoch": 0}, open_context=lambda *args: live)
                with mock.patch.object(backup, "_application_context", return_value=(ops, object())), \
                        mock.patch.object(backup,"reserve_capture",side_effect=backup.PaperBackupError("PAPER_BACKUP_RESERVATION_REQUIRED")) as reserve:
                    if expected is None:
                        self.assertIsNone(backup.run_locked(object(), 3, "verify", 10))
                    else:
                        with self.assertRaisesRegex(backup.PaperBackupError, expected):
                            backup.run_locked(object(), 3, "verify", 10)
                live.close.assert_called_once()
                self.assertEqual(reserve.call_count, int(configured))

    def test_exchange_drains_owned_child_while_writing_and_keeps_marker_private(self):
        # Real owned pipe/child exercise only; this is not a PostgreSQL fixture.
        import subprocess, time
        started = time.monotonic()
        def remaining(cap=15):
            left = 10 - (time.monotonic() - started)
            if left <= 0:
                raise backup.PaperBackupError("PAPER_BACKUP_LIMIT")
            return min(cap, left)
        owner = types.SimpleNamespace(maximum=2*1024*1024,
            _custody=types.SimpleNamespace(deadline=types.SimpleNamespace(remaining=remaining, work_remaining=remaining)),
            revalidate=mock.Mock())
        client = backup._Client(owner, "component")
        program = "import sys\nfor line in sys.stdin.buffer:\n if line.startswith(b'\\echo '):\n  sys.stdout.buffer.write(line[6:]);sys.stdout.buffer.flush()\n elif line.strip():\n  sys.stdout.buffer.write(b'x'*4096+b'\\n');sys.stdout.buffer.flush()\n"
        client.child = subprocess.Popen([sys.executable, "-I", "-S", "-B", "-c", program],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        try:
            stream = io.BytesIO()
            total = client.exchange((b"input"*1024+b"\n")*64, stream)
            self.assertEqual(total, 64*4097)
            self.assertEqual(stream.getvalue(), (b"x"*4096+b"\n")*64)
            self.assertNotIn(b"PAPER_CAPTURE_", stream.getvalue())
        finally:
            client.child.stdin.close()
            try:
                client.child.wait(timeout=2)
            except subprocess.TimeoutExpired:
                client.child.terminate(); client.child.wait(timeout=2)
            client.child.stdout.close()

    def test_sealed_spool_rejects_same_bytes_named_replacement(self):
        import hashlib
        owner = object.__new__(backup._HeldPaperSnapshot)
        owner.maximum, owner.spools = 4096, {}
        identity = lambda v: (v.st_dev,v.st_ino,v.st_mode,v.st_uid,v.st_gid,v.st_nlink,v.st_size,v.st_mtime_ns,v.st_ctime_ns)
        owner.selection = types.SimpleNamespace(_backup_module=lambda: types.SimpleNamespace(files=types.SimpleNamespace(identity=identity)))
        def hashed(fd):
            raw = os.pread(fd,4097,0)
            return len(raw), "sha256:"+hashlib.sha256(raw).hexdigest()
        owner.ops = types.SimpleNamespace(_hash_fd=hashed)
        with tempfile.TemporaryDirectory() as root:
            owner.root = Path(root)
            directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            owner.root_tree = types.SimpleNamespace(fd=directory)
            try:
                with owner._new_spool("audit.stream") as stream:
                    stream.write(b"original-audit")
                    owner._seal_spool("audit.stream", stream)
                    owner._check_spools()
                    self.assertEqual(os.stat(owner.root / "audit.stream").st_mode & 0o777, 0o400)
                    (owner.root / "replacement").write_bytes(b"original-audit")
                    os.replace(owner.root / "replacement", owner.root / "audit.stream")
                    with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_SPOOL_CHANGED"):
                        owner._check_spools()
            finally:
                for fd,*_ in owner.spools.values():
                    os.close(fd)
                os.close(directory)

    def test_client_exit_is_not_live_transaction_authority(self):
        owner = types.SimpleNamespace(context=types.SimpleNamespace(docker=mock.Mock()))
        client = backup._Client(owner, "exporter")
        client.child = types.SimpleNamespace(poll=lambda: 0)
        client.generation = {"captured": True}
        with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_CLIENT_LOST"):
            client.check()
        owner.context.docker.object.assert_not_called()

    def test_successful_stop_return_is_not_client_terminal_proof(self):
        docker = mock.Mock()
        docker.object.return_value = {"State": {"Running": True}}
        ops = types.SimpleNamespace(_container_generation=lambda value: {"captured": True},
            _terminal_container=mock.Mock(side_effect=backup.PaperBackupError("PAPER_BACKUP_CLEANUP_UNKNOWN")))
        owner = types.SimpleNamespace(context=types.SimpleNamespace(docker=docker), ops=ops,
            _cleanup_docker=mock.Mock(return_value=""))
        client = backup._Client(owner, "exporter")
        client.sent, client.cid, client.created = True, "captured-cid", "captured-created"
        client.generation = {"captured": True}
        with mock.patch.object(client, "policy"), self.assertRaises(backup.PaperBackupError):
            client.close()
        self.assertFalse(client.absent)
        self.assertEqual(owner._cleanup_docker.call_args_list, [mock.call(["stop", "--time", "5", "captured-cid"])])

    def test_no_process_in_the_client_cgroup_is_admitted_as_a_child(self):
        owner, _unused, _row = self.owner()
        client = backup._Client(owner, "exporter")
        client.process = {"pid": 40, "start_ticks": 50, "parent": 30, "cgroup": "/owned",
                          "executable": {"bytes": 8, "sha256": "psql", "device": 1, "inode": 2}}
        client.executables = {"/usr/lib/postgresql/18/bin/pg_dump": (7, "dump"), "/bin/sh": (9, "sh")}
        child = {"pid": 44, "start_ticks": 55, "parent": 40, "cgroup": "/owned",
                 "executable": {"bytes": 7, "sha256": "dump", "device": 1, "inode": 3}}
        owner.ops._process_record = mock.Mock(return_value=child)
        owner.ops._proc_cgroups = lambda: {40: "/owned", 44: "/foreign"}
        self.assertEqual(client.child_processes(), {})
        owner.ops._proc_cgroups = lambda: {40: "/owned", 44: "/owned"}
        for executable in (client.process["executable"], child["executable"]):  # psql fork, pinned pg_dump ELF
            owner.ops._process_record.return_value = dict(child, executable=executable)
            with self.subTest(executable=executable["sha256"]), \
                    self.assertRaisesRegex(backup.PaperBackupError, "UNOWNED_CHILD"):
                client.child_processes()

    def test_client_disposal_requires_independent_absence_of_its_backend(self):
        owner, _unused, exporter = self.owner()
        client = backup._Client(owner, "exporter")
        client.backend = exporter
        client.sent, client.cid, client.created = True, "owned", "created"
        client.generation = ("owned-generation",)
        owner.ops._container_generation = lambda value: client.generation
        owner.ops._terminal_container = lambda value,generation: "terminal"
        owner.ops._proc_cgroups = lambda: {}
        owner.context.docker.object.return_value = {"State":{"Running":False}}
        owner._cleanup_docker = mock.Mock(return_value="")
        owner._custody = types.SimpleNamespace(
            observer=types.SimpleNamespace(database=mock.Mock(return_value={"backends":[exporter]})))
        with mock.patch.object(client,"policy"), self.assertRaisesRegex(backup.PaperBackupError,"CLEANUP_UNKNOWN"):
            client.close()
        owner._cleanup_docker.assert_not_called()
        self.assertFalse(client.absent)
        owner._custody.observer.database.return_value = {"backends":[]}
        with mock.patch.object(client,"policy"):
            client.close()
        self.assertTrue(client.absent)
        self.assertEqual(owner._cleanup_docker.call_args_list[0], mock.call(["rm","owned"]))


class ExchangeCleanupTests(unittest.TestCase):
    def client(self):
        owner = types.SimpleNamespace(maximum=10000,
            _custody=types.SimpleNamespace(deadline=types.SimpleNamespace(work_remaining=lambda cap=15: cap)))
        client = backup._Client(owner, "exporter")
        client.backend = {"pid": 10}
        client.child = types.SimpleNamespace(poll=lambda: None,
            stdin=types.SimpleNamespace(fileno=lambda: 3), stdout=types.SimpleNamespace(fileno=lambda: 4))
        return client

    def test_primary_failure_survives_failed_fd_restore(self):
        for primary in (backup.PaperBackupError("PRIMARY"), KeyboardInterrupt()):
            with self.subTest(primary=type(primary).__name__):
                client = self.client()
                def blocking(fd, enabled):
                    if enabled:
                        raise OSError("RESTORE")
                with mock.patch.object(backup.select, "select", side_effect=primary), \
                        mock.patch.object(backup.os, "set_blocking", side_effect=blocking) as restore, \
                        self.assertRaises(type(primary)) as caught:
                    client.exchange(b"actual-source-command")
                self.assertIs(caught.exception, primary)
                self.assertEqual(restore.call_args_list[-2:], [mock.call(3, True), mock.call(4, True)])

    def test_first_fd_restore_error_is_preserved_and_second_restore_is_attempted(self):
        client = self.client()
        first, second = OSError("FIRST_RESTORE"), OSError("SECOND_RESTORE")
        written = []
        def write(fd, data):
            written.append(bytes(data))
            return len(data)
        def read(fd, size):
            marker = written[0].split(b"\\echo ")[1].splitlines()[0]
            return marker + b"\n"
        def blocking(fd, enabled):
            if enabled:
                raise first if fd == 3 else second
        with mock.patch.object(backup.os, "set_blocking", side_effect=blocking) as restore, \
                mock.patch.object(backup.os, "write", side_effect=write), \
                mock.patch.object(backup.os, "read", side_effect=read), \
                mock.patch.object(backup.select, "select", return_value=([4], [3], [])), \
                self.assertRaises(OSError) as caught:
            client.exchange(b"actual-source-command")
        self.assertIs(caught.exception, first)
        self.assertEqual(restore.call_args_list[-2:], [mock.call(3, True), mock.call(4, True)])


class PaperRetainedStorageTests(unittest.TestCase):
    """Actual temporary files/publication; root ownership is mocked, not granted.

    The member bytes are synthetic storage inputs, not encrypted/native archive
    acceptance. No _PaperPublication or verifier authority is constructed here.
    """
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.ops = backup._local("paper_storage_test_ops", "owner-beta-paper-operations.py")
        self.daily = backup._local("paper_storage_test_daily", "owner-beta-daily-backup.py")
        self.stack = contextlib.ExitStack()
        self.addCleanup(self.stack.close)
        for target in ("subprocess.Popen", "os.chown", "os.fchown"):
            self.stack.enter_context(mock.patch(target, side_effect=AssertionError("forbidden local effect")))
        ops = self.ops

        class TemporaryTree:
            def __init__(self, path):
                self.path = path
                self.fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
                self.identity = ops._identity(os.fstat(self.fd))

            def revalidate(self):
                backup._require(ops._identity(os.fstat(self.fd)) == self.identity
                    and ops._identity(os.stat(self.path, follow_symlinks=False)) == self.identity,
                    "PAPER_BACKUP_SET_CHANGED")

            def close(self):
                os.close(self.fd)

        @contextlib.contextmanager
        def temporary_directory(path):
            tree = TemporaryTree(path)
            try:
                yield tree.fd
            finally:
                tree.close()

        original_file = ops._file
        def temporary_file(fd, mode=0o600, *, uid=0, gid=0):
            return original_file(fd, mode, uid=os.geteuid(), gid=os.getegid())

        original_directory = self.daily.install.directory_info
        def temporary_directory_info(fd, uid=0, gid=None, mode=None):
            return original_directory(fd, uid=os.geteuid(), gid=os.getegid(), mode=mode)

        original_stat = os.stat
        def temporary_latest_owner(path, *args, **kwargs):
            info = original_stat(path, *args, **kwargs)
            if path == "latest" and kwargs.get("follow_symlinks") is False:
                fields = list(info)
                fields[4] = 0  # Only the existing symlink's root-owner check is mocked.
                return os.stat_result(fields)
            return info

        self.stack.enter_context(mock.patch.object(ops.journal, "_Tree", TemporaryTree))
        self.stack.enter_context(mock.patch.object(ops, "_file", temporary_file))
        self.stack.enter_context(mock.patch.object(self.daily.install, "root_directory", temporary_directory))
        self.stack.enter_context(mock.patch.object(self.daily.install, "directory_info", temporary_directory_info))
        self.stack.enter_context(mock.patch.object(self.daily.install, "file_info",
            side_effect=lambda fd, uid, gid, mode: temporary_file(fd, mode)))
        self.stack.enter_context(mock.patch.object(os, "stat", temporary_latest_owner))

    def storage(self, commit="b" * 40):
        owner = types.SimpleNamespace(ops=self.ops, context=types.SimpleNamespace(commit=commit),
            maximum=1024 * 1024, selection=types.SimpleNamespace(_backup_module=lambda: self.daily))
        return backup._PaperStorage(owner, {"BACKUP_ROOT": str(self.root), "MAX_TOTAL_BYTES": owner.maximum})

    def candidate(self, storage, stamp="20261111T010000Z", *, commit=None):
        commit = storage.context.commit if commit is None else commit
        path = self.root / (".paper-capture-" + stamp)
        path.mkdir(mode=0o700)
        hashes = {}
        for number, name in enumerate(backup._PAPER_MEMBERS):
            data = b"SYNTHETIC storage member\x00" * (number + 1)
            (path / name).write_bytes(data)
            (path / name).chmod(0o600)
            hashes[name] = "sha256:" + backup.hashlib.sha256(data).hexdigest()
        metadata = dict(format=backup._PAPER_FORMAT, commit=commit, created_at=stamp,
            encryption="AES-256-CBC-PBKDF2-200000", files=hashes)
        self.write_metadata(path, metadata)
        name = storage.prefix + stamp + "-" + commit
        return path, name

    def write_metadata(self, path, value):
        (path / "metadata.json").write_bytes(self.ops._canonical(value))
        (path / "metadata.json").chmod(0o600)

    @staticmethod
    def contents(path):
        return {item.name: item.read_bytes() for item in path.iterdir()}

    @staticmethod
    def size(path):
        return sum(item.stat().st_size for item in path.iterdir())

    def first_publication(self, storage):
        source, name = self.candidate(storage)
        final = self.root / name
        storage.publish(source, final, name)
        return final

    def nonpaper_sets(self, *, kinds=("daily", "simulation", "preactivation", "inactive-simulation", "legacy")):
        # Fixtures for the existing retained-inventory reader, not restore or
        # encryption acceptance. Future dates must not affect the Paper clock.
        markers = {"daily": self.daily.FORMAT, "simulation": self.daily.SIMULATION_FORMAT,
            "preactivation": self.daily.PREACTIVATION_FORMAT,
            "inactive-simulation": self.daily.INACTIVE_SIMULATION_FORMAT,
            "legacy": "LAGRANGE_PRODUCTION_BACKUP_V1"}
        paths = []
        for kind in kinds:
            name = "backup-" + (kind + "-" if kind != "legacy" else "") + "20261112T010000Z-" + "a" * 40
            path = self.root / name
            path.mkdir(mode=0o700)
            if kind == "legacy":
                values = {member: b"SYNTHETIC retained legacy member\x00" * (index + 1)
                    for index, member in enumerate(("postgres.dump.enc", "raw.tar.enc", "curated.tar.enc"))}
                values["manifest.sha256"] = b"".join((backup.hashlib.sha256(data).hexdigest() + "  " + member + "\n").encode()
                    for member, data in values.items())
                values["metadata"] = b"SYNTHETIC retained inventory fixture\n"
                values["VERIFIED"] = b"2026-11-12T01:00:00Z\n"
            else:
                values = {member: b"SYNTHETIC retained member\x00" * (index + 1)
                    for index, member in enumerate(self.daily.MEMBERS)}
                values["metadata.json"] = self.ops._canonical(dict(format=markers[kind], commit="a" * 40,
                    created_at="20261112T010000Z", encryption="AES-256-CBC-PBKDF2-200000",
                    files={member: "sha256:" + backup.hashlib.sha256(data).hexdigest()
                           for member, data in values.items()}))
            values["COMPLETE"] = (markers[kind] + "\n").encode()
            for member, data in values.items():
                (path / member).write_bytes(data)
                (path / member).chmod(0o600)
            paths.append(path)
        return paths

    def test_release_b_inventories_and_publishes_after_retained_release_a(self):
        old = self.first_publication(self.storage("a" * 40))
        original = self.contents(old)
        current = self.storage("b" * 40)
        self.assertEqual(current.sets()[0]["size"], self.size(old))
        source, name = self.candidate(current, "20261111T010001Z")
        current.publish(source, self.root / name, name)
        self.assertEqual(self.contents(old), original)
        self.assertEqual([row["name"] for row in current.sets()], [old.name, name])
        self.assertEqual([row["size"] for row in current.sets()], [self.size(old), self.size(self.root / name)])
        self.assertEqual(os.readlink(self.root / "latest"), name)
        retained = current.latest()
        try:
            self.assertEqual(retained.metadata["commit"], "b" * 40)
        finally:
            retained.close()

    def test_historical_set_requires_own_canonical_identity_and_member_hashes(self):
        old = self.first_publication(self.storage("a" * 40))
        original = self.contents(old)
        metadata = backup.json.loads(original["metadata.json"])
        current = self.storage("b" * 40)
        mutations = [("commit", value) for value in (None, 123, "A" * 40, "0" * 40, "a" * 39, "b" * 40)]
        mutations += [("created_at", value) for value in (None, 123, "20260230T010000Z", "20261111T010001Z")]
        mutations += [("files", {**metadata["files"], backup._PAPER_MEMBERS[0]: "sha256:" + "0" * 64})]
        for field, value in mutations:
            with self.subTest(field=field, value=value):
                self.write_metadata(old, {**metadata, field: value})
                with self.assertRaises(backup.PaperBackupError):
                    current.sets()
        self.write_metadata(old, metadata)
        (old / backup._PAPER_MEMBERS[0]).write_bytes(b"changed ciphertext")
        with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_CIPHER_DIGEST"):
            current.sets()
        (old / backup._PAPER_MEMBERS[0]).write_bytes(original[backup._PAPER_MEMBERS[0]])
        (old / "unexpected").write_bytes(b"extra")
        with self.assertRaisesRegex(self.daily.runtime.RuntimeErrorCode, "DAILY_BACKUP_DIRECTORY_LIMIT"):
            current.sets()
        (old / "unexpected").unlink()
        (old / backup._PAPER_MEMBERS[0]).unlink()
        with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_SET_MEMBERS"):
            current.sets()

    def test_historical_metadata_stays_canonical_and_retained_members_stay_bound(self):
        old = self.first_publication(self.storage("a" * 40))
        current = self.storage()
        metadata = (old / "metadata.json").read_bytes()
        (old / "metadata.json").write_bytes(metadata + b"\n")
        with self.assertRaisesRegex(self.ops.journal.JournalError, "JOURNAL_CANONICAL"):
            current.sets()
        (old / "metadata.json").write_bytes(metadata)
        retained = backup._StoredPaperSet(current, old)
        try:
            member = old / backup._PAPER_MEMBERS[0]
            content = member.read_bytes()
            member.unlink()
            member.write_bytes(content)
            member.chmod(0o600)
            with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_SET_CHANGED"):
                retained.revalidate()
        finally:
            retained.close()

    def test_private_candidate_still_requires_current_release(self):
        current = self.storage()
        source, name = self.candidate(current, commit="a" * 40)
        with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_METADATA"):
            backup._StoredPaperSet(current, source, completed=False)
        with mock.patch.object(self.daily.Storage, "publish") as writer:
            with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_METADATA"):
                current.publish(source, self.root / name, name)
        writer.assert_not_called()
        self.assertFalse((source / "COMPLETE").exists())
        self.assertFalse((self.root / "latest").is_symlink())

    def test_candidate_publication_name_must_match_metadata(self):
        current = self.storage()
        source, name = self.candidate(current)
        wrong = name.replace("010000Z", "010001Z")
        with mock.patch.object(self.daily.Storage, "publish") as writer:
            with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_SET_NAME"):
                current.publish(source, self.root / wrong, wrong)
        writer.assert_not_called()
        self.assertFalse((source / "COMPLETE").exists())

    def test_aggregate_cap_refuses_before_complete_or_latest_and_preserves_prior_set(self):
        current = self.storage()
        old = self.first_publication(current)
        original = self.contents(old)
        latest = (self.root / "latest").lstat()
        source, name = self.candidate(current, "20261111T010001Z")
        candidate = self.contents(source)
        total = self.size(old) + self.size(source) + len((backup._PAPER_FORMAT + "\n").encode())
        current.values["MAX_TOTAL_BYTES"] = total - 1
        self.assertLess(self.size(old), current.values["MAX_TOTAL_BYTES"])
        self.assertLess(self.size(source), current.values["MAX_TOTAL_BYTES"])
        with mock.patch.object(self.daily.Storage, "publish", wraps=self.daily.Storage.publish) as writer:
            with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_RETENTION_CAP"):
                current.publish(source, self.root / name, name)
        writer.assert_not_called()
        self.assertEqual(self.contents(old), original)
        self.assertEqual(self.contents(source), candidate)
        self.assertFalse((source / "COMPLETE").exists())
        self.assertFalse((self.root / name).exists())
        self.assertEqual(os.readlink(self.root / "latest"), old.name)
        self.assertEqual((self.root / "latest").lstat(), latest)

    def test_exact_aggregate_cap_includes_both_metadata_and_complete_files(self):
        current = self.storage()
        old = self.first_publication(current)
        source, name = self.candidate(current, "20261111T010001Z")
        total = self.size(old) + self.size(source) + len((backup._PAPER_FORMAT + "\n").encode())
        current.values["MAX_TOTAL_BYTES"] = total
        current.publish(source, self.root / name, name)
        self.assertEqual(sum(row["size"] for row in current.sets()), total)
        self.assertEqual(self.size(old) + self.size(self.root / name), total)
        self.assertEqual(os.readlink(self.root / "latest"), name)
        self.assertEqual((self.root / name / "COMPLETE").read_bytes(), (backup._PAPER_FORMAT + "\n").encode())

    def test_mixed_root_cap_counts_every_recognized_nonpaper_format_before_publication(self):
        current = self.storage()
        old = self.first_publication(current)
        nonpaper = self.nonpaper_sets()
        retained = [old, *nonpaper]
        before = {path.name: self.contents(path) for path in retained}
        (self.root / "latest").unlink()
        (self.root / "latest").symlink_to(nonpaper[-1].name)
        latest = (self.root / "latest").lstat()
        self.assertEqual({row["name"] for row in self.daily.Storage.sets(current)}, {path.name for path in nonpaper})
        source, name = self.candidate(current, "20261111T010001Z")
        private = self.contents(source)
        total = sum(self.size(path) for path in retained) + self.size(source) + len((backup._PAPER_FORMAT + "\n").encode())
        current.values["MAX_TOTAL_BYTES"] = total - 1
        self.assertLess(self.size(old) + self.size(source) + len((backup._PAPER_FORMAT + "\n").encode()), total - 1)
        with mock.patch.object(self.daily.Storage, "publish", wraps=self.daily.Storage.publish) as writer:
            with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_RETENTION_CAP"):
                current.publish(source, self.root / name, name)
        writer.assert_not_called()
        self.assertEqual({path.name: self.contents(path) for path in retained}, before)
        self.assertEqual(self.contents(source), private)
        self.assertFalse((source / "COMPLETE").exists())
        self.assertFalse((self.root / name).exists())
        self.assertEqual(os.readlink(self.root / "latest"), nonpaper[-1].name)
        self.assertEqual((self.root / "latest").lstat(), latest)

    def test_mixed_root_exact_cap_succeeds_without_mixing_paper_clock_or_latest_inventory(self):
        current = self.storage()
        old = self.first_publication(current)
        nonpaper = self.nonpaper_sets()
        before = {path.name: self.contents(path) for path in nonpaper}
        (self.root / "latest").unlink()
        (self.root / "latest").symlink_to(nonpaper[-1].name)
        source, name = self.candidate(current, "20261111T010001Z")
        total = self.size(old) + sum(self.size(path) for path in nonpaper) + self.size(source) + len((backup._PAPER_FORMAT + "\n").encode())
        current.values["MAX_TOTAL_BYTES"] = total
        current.publish(source, self.root / name, name)
        self.assertEqual(sum(row["size"] for row in current.sets())
                         + sum(row["size"] for row in self.daily.Storage.sets(current)), total)
        self.assertEqual([row["name"] for row in current.sets()], [old.name, name])
        self.assertEqual({path.name: self.contents(path) for path in nonpaper}, before)
        self.assertEqual(os.readlink(self.root / "latest"), name)
        latest = current.latest()
        try:
            self.assertEqual(latest.path.name, name)
        finally:
            latest.close()

    def test_mixed_root_malformed_or_unreadable_nonpaper_set_refuses_before_publication(self):
        current = self.storage()
        retained = self.nonpaper_sets(kinds=("simulation",))[0]
        (self.root / "latest").symlink_to(retained.name)
        latest = (self.root / "latest").lstat()
        source, name = self.candidate(current)
        private = self.contents(source)
        original = self.contents(retained)
        for fault in ("marker", "missing", "unreadable"):
            with self.subTest(fault=fault):
                if fault == "marker":
                    (retained / "COMPLETE").write_bytes(b"wrong marker\n")
                elif fault == "missing":
                    (retained / "postgres.dump.enc").unlink()
                before = self.contents(retained)
                with contextlib.ExitStack() as stack:
                    if fault == "unreadable":
                        stack.enter_context(mock.patch.object(self.daily.install, "read_file", side_effect=PermissionError()))
                    writer = stack.enter_context(mock.patch.object(self.daily.Storage, "publish"))
                    error = PermissionError if fault == "unreadable" else self.daily.runtime.RuntimeErrorCode
                    with self.assertRaises(error):
                        current.publish(source, self.root / name, name)
                    writer.assert_not_called()
                self.assertEqual(self.contents(retained), before)
                self.assertEqual(self.contents(source), private)
                self.assertFalse((source / "COMPLETE").exists())
                self.assertFalse((self.root / name).exists())
                self.assertEqual(os.readlink(self.root / "latest"), retained.name)
                self.assertEqual((self.root / "latest").lstat(), latest)
                for member, data in original.items():
                    (retained / member).write_bytes(data)
                    (retained / member).chmod(0o600)


class PaperLifecycleOrderTests(unittest.TestCase):
    def run_route(self, action="backup", fault=None):
        import contextlib
        events=[]
        identity="11111111-1111-4111-8111-111111111111"
        operation=mock.Mock(intent_sha256="sha256:"+"a"*64)
        store=mock.Mock(epoch_floor=0,installation_id=identity)
        store.bind_caller.return_value=operation
        invocation=mock.MagicMock(result={"outcome":"OK"})
        invocation.__enter__.return_value=invocation
        inventory=types.SimpleNamespace(_presence=object(),_configured=True,_pages=[(None,b"page")],
            _local=types.SimpleNamespace(operations=[]),revalidate=mock.Mock())
        live=mock.Mock(manifest_sha256="sha256:"+"b"*64)
        live.probe_history.side_effect=lambda:contextlib.nullcontext(inventory)
        live.invoke.return_value=invocation
        live.close.side_effect=lambda:events.append("context-close")
        candidate=mock.Mock(generation=identity,archive_sha256="sha256:"+"c"*64)
        def publish():
            events.append("publish")
            if fault=="rename": raise OSError("rename uncertainty")
        candidate.publish.side_effect=publish
        held=mock.Mock()
        held.selection._backup_module.return_value.files.Limits.side_effect=lambda maximum:maximum
        held.capture.side_effect=lambda *_args:(events.append("capture") or candidate)
        held.release_capture_clients.side_effect=lambda:events.append("clients-closed")
        def verify(*_args):
            events.append("fresh-verify")
            if fault in {"verify","cancel","reap"}:
                raise backup.PaperBackupError("PAPER_BACKUP_CLEANUP_UNKNOWN")
        held.verify_candidate.side_effect=verify
        held._custody.seal_snapshot.side_effect=lambda *_args:events.append("SNAPSHOT")
        def close_owner():
            events.append("owner-close")
            if fault=="source-cleanup": raise backup.PaperBackupError("PAPER_BACKUP_CLEANUP_UNKNOWN")
        held.close.side_effect=close_owner
        space=mock.Mock(owner=held)
        space.close.side_effect=lambda:events.append("space-close")
        ops=mock.Mock(_HISTORY_EMPTY=object(),MAX_PAGE=4096)
        ops.open_context.return_value=live
        ops._json.return_value={"account_id":identity,"control_epoch":0}
        ops._canonical.side_effect=lambda value:backup.json.dumps(value).encode()
        ops._current_boot_id.return_value=identity
        ops._stage_observation.return_value={"realtime_ns":1,"monotonic_ns":1,"boottime_ns":1}
        operation.append.return_value="sha256:"+"d"*64
        ops.journal.open_journal.side_effect=lambda *_a,**_kw:contextlib.nullcontext(store)
        patches=[mock.patch.object(backup,"_application_context",return_value=(ops,object())),
            mock.patch.object(backup,"reserve_capture",return_value=space),
            mock.patch.object(backup,"_latest_predecessor",return_value=None),
            mock.patch.object(backup,"open_snapshot",side_effect=lambda *_a:contextlib.nullcontext(held))]
        with contextlib.ExitStack() as stack:
            for patch in patches:stack.enter_context(patch)
            if fault is None:
                result=backup.run_locked(object(),7,action,1000,storage_values={})
                self.assertEqual(result.status,"BACKUP_COMPLETE")
            else:
                with self.assertRaises((backup.PaperBackupError,OSError)):
                    backup.run_locked(object(),7,action,1000,storage_values={})
        for run in list(backup._RUNS):
            if run.live is live:backup._RUNS.discard(run)
        return events,candidate,held

    def test_fresh_candidate_reverify_and_all_cleanup_precede_snapshot_publication(self):
        events,candidate,_held=self.run_route()
        for before,after in (("clients-closed","fresh-verify"),("fresh-verify","SNAPSHOT"),
                             ("SNAPSHOT","owner-close"),("context-close","publish"),("space-close","publish")):
            self.assertLess(events.index(before),events.index(after))
        candidate.publish.assert_called_once()

    def test_failed_cancelled_or_unknown_reap_candidate_never_seals_or_publishes(self):
        for fault in ("verify","cancel","reap"):
            with self.subTest(fault=fault):
                events,candidate,held=self.run_route(fault=fault)
                self.assertNotIn("SNAPSHOT",events)
                candidate.publish.assert_not_called()
                held._custody.seal_snapshot.assert_not_called()

    def test_source_cleanup_failure_prevents_complete_and_latest(self):
        _events,candidate,_held=self.run_route(fault="source-cleanup")
        candidate.publish.assert_not_called()

    def test_rename_uncertainty_preserves_candidate_without_replaying_capture(self):
        events,candidate,held=self.run_route(fault="rename")
        self.assertLess(events.index("space-close"),events.index("publish"))
        held.capture.assert_called_once()
        held.verify_candidate.assert_called_once()
        candidate.publish.assert_called_once()
        candidate.close.assert_called_once()

    def test_standalone_verify_has_no_source_mutation_or_drain(self):
        import contextlib
        live=mock.Mock()
        inventory=types.SimpleNamespace(_presence=object(),_configured=True,_pages=[(None,b"page")],
            _local=types.SimpleNamespace(operations=[]),revalidate=mock.Mock())
        live.probe_history.side_effect=lambda:contextlib.nullcontext(inventory)
        owner=mock.Mock()
        owner.selection._backup_module.return_value.files.Limits.side_effect=lambda maximum:maximum
        space=mock.Mock(owner=owner)
        persisted=mock.Mock()
        storage=mock.Mock()
        storage.latest.return_value=persisted
        ops=mock.Mock(_HISTORY_EMPTY=object(),MAX_PAGE=4096)
        ops.open_context.return_value=live
        ops._json.return_value={"account_id":"11111111-1111-4111-8111-111111111111","control_epoch":8}
        with mock.patch.object(backup,"_application_context",return_value=(ops,object())), \
                mock.patch.object(backup,"reserve_capture",return_value=space), \
                mock.patch.object(backup,"_PersistedArchiveOwner",return_value=owner), \
                mock.patch.object(backup,"_PaperStorage",return_value=storage), \
                mock.patch.object(backup,"_latest_predecessor") as predecessor:
            self.assertEqual(backup.run_locked(object(),7,"verify",1000,storage_values={}).status,"VERIFIED_ISOLATED")
        owner.verify_persisted.assert_called_once_with(persisted,1000,{})
        live.invoke.assert_not_called();live.drain.assert_not_called()
        ops.commit_off.assert_not_called();ops.journal.open_journal.assert_not_called()
        ops.daily.Database.assert_not_called();predecessor.assert_not_called()
        persisted.close.assert_called_once()

    def test_daily_repair_rejects_economic_or_existing_admission_change(self):
        before={("cash_ledger","x"):{"amount":"1"},
                ("owner_beta_daily_admissions","a"):{"admission_sha256":"a","content_sha256":"original"}}
        facts={"extra_admissions":[],"expected_admissions":["a"]}
        backup._check_daily_repair_delta(before,dict(before),facts)
        for key in before:
            after=copy.deepcopy(before)
            after[key][next(iter(after[key]))]="corrupt"
            with self.subTest(key=key),self.assertRaises(backup.PaperBackupError):
                backup._check_daily_repair_delta(before,after,facts)


class CaptureSpaceConstructionTests(unittest.TestCase):
    def test_capture_space_uses_the_parent_tree_without_a_context_manager(self):
        # journal._Tree offers fd, revalidate and close only. Using it in a with-statement raised
        # TypeError, so the BACKUP case could never reserve its capture space.
        events = []
        with tempfile.TemporaryDirectory() as directory:
            class Tree:
                def __init__(self, path):
                    events.append(("open", path))
                    self.path = path
                    self.fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY) if path == "/run" else -1

                def close(self):
                    events.append(("close", self.path))
                    if self.fd >= 0:
                        os.close(self.fd)
                        self.fd = -1

            class Context:
                def revalidate(self):
                    events.append(("revalidate", None))

            fake_ops = types.SimpleNamespace(
                _STOP_ATTEMPTS={}, _ArchiveDeadline=lambda: object(),
                journal=types.SimpleNamespace(_Tree=Tree))
            selection = types.SimpleNamespace(
                _mkdir_private_tree=lambda files: "tree",
                _backup_module=lambda: types.SimpleNamespace(files=object()),
                _dispose_private_tree=lambda files, tree: events.append(("dispose", None)))
            context = Context()
            with mock.patch.object(backup, "_operations", return_value=fake_ops), \
                 mock.patch.object(backup, "_local", return_value=selection):
                space = backup._CaptureSpace(context)
            try:
                self.assertIs(backup._SPACES.get(context), space)
                self.assertEqual(space.root.parent, Path("/run"))
                self.assertEqual(os.listdir(directory), [space.root.name])
                self.assertEqual(os.stat(os.path.join(directory, space.root.name)).st_mode & 0o777, 0o700)
                self.assertEqual([event for event in events if event[0] in ("open", "close")],
                                 [("open", "/run"), ("open", str(space.root)), ("close", "/run")])
                self.assertNotIn(("dispose", None), events)
            finally:
                backup._SPACES.pop(context, None)



class PaperDumpArgvTests(unittest.TestCase):
    CONTAINER = "0123456789abcdef" * 4
    SNAPSHOT = "00000003-00000002-1"
    APPLICATION = "lagrange_paper_dump_" + "0123456789abcdef" * 2

    def argv(self, **changes):
        values = {"container_id": self.CONTAINER, "role": "lagrange", "database": "lagrange",
                  "snapshot": self.SNAPSHOT, "application_name": self.APPLICATION}
        values.update(changes)
        return backup.paper_dump_argv(**values)

    def test_argv_is_exact(self):
        self.assertEqual(self.argv(), [
            "/usr/bin/docker", "--host", "unix:///var/run/docker.sock", "exec", "--user", "999:999",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "/usr/bin/env", "-i", "PATH=/usr/lib/postgresql/18/bin:/usr/bin:/bin",
            "PGHOST=/var/run/postgresql", "PGCONNECT_TIMEOUT=10", "PGPASSFILE=/dev/null",
            "PGSERVICEFILE=/dev/null", "PGCLIENTENCODING=UTF8",
            "PGAPPNAME=lagrange_paper_dump_0123456789abcdef0123456789abcdef",
            "PGOPTIONS=-c default_transaction_read_only=on -c statement_timeout=900000 "
            "-c lock_timeout=5000 -c max_parallel_workers_per_gather=0",
            "/usr/bin/timeout", "--signal=TERM", "--kill-after=5s", "900s",
            "/usr/lib/postgresql/18/bin/pg_dump", "--format=custom", "--no-password",
            "--host=/var/run/postgresql", "--username=lagrange", "--dbname=lagrange",
            "--snapshot=00000003-00000002-1"])

    def test_every_input_is_validated(self):
        for name, value in (
                ("container_id", "a" * 63), ("container_id", "A" * 64), ("container_id", None),
                ("role", "lagrange; DROP"), ("role", ""), ("role", 1),
                ("database", "lagrange db"),
                ("snapshot", "abc; x"), ("snapshot", ""), ("snapshot", "a" * 65),
                ("application_name", "0123456789abcdef" * 2),
                ("application_name", "lagrange_paper_dump_" + "a" * 31),
                ("application_name", "lagrange_paper_dump_" + "A" * 32),
                ("application_name", None)):
            with self.subTest(name=name, value=value):
                with self.assertRaises(backup.PaperBackupError) as raised:
                    self.argv(**{name: value})
                self.assertEqual(raised.exception.args, ("PAPER_BACKUP_DUMP_ARGV",))

    def test_no_shell_and_no_credentials(self):
        argv = self.argv()
        self.assertEqual([item for item in argv if item in ("sh", "-c", "bash")], [])
        self.assertEqual(argv[argv.index("/usr/bin/env") + 1], "-i")
        self.assertEqual([item for item in argv if "PASSWORD" in item], [])


class LocalDumpAdmissionTests(unittest.TestCase):
    """Real _match_clients/_LocalDump.admit on object.__new__ owners; rows as the archive observer reports them."""

    def owner(self):
        owner = object.__new__(backup._HeldPaperSnapshot)
        exporter = {"pid": 10, "start_us": 20, "role": "migration_owner", "addr": "10.0.0.2", "port": 30100}
        owner.clients = [types.SimpleNamespace(backend=exporter, absent=False, dumping=False)]
        dump = backup._LocalDump(owner)
        dump.sent, dump.role, dump.started_us = True, "lagrange", 5_000_000
        owner.local_dump = dump
        session = {"pid": 11, "start_us": 4_000_000, "role": "lagrange", "addr": None, "port": -1,
                   "app": dump.application_name}
        return owner, dump, dict(exporter, app="psql"), session

    def test_dump_session_is_admitted_and_fixed_at_first_sight(self):
        owner, dump, exporter, session = self.owner()
        owner._match_clients([exporter, session])
        self.assertEqual(dump.session, session)
        owner._match_clients([exporter, dict(session)])
        owner._match_clients([exporter])  # the dump may end; the exporter must remain
        self.assertEqual(dump.session, session)

    def test_every_other_session_is_refused(self):
        cases = {"role": ({"role": "migration_owner"}, "PAPER_BACKUP_FOREIGN_CLIENT"),
                 "address": ({"addr": "10.0.0.5"}, "PAPER_BACKUP_FOREIGN_CLIENT"),
                 "port": ({"port": 5432}, "PAPER_BACKUP_FOREIGN_CLIENT"),
                 "application": ({"app": "lagrange_paper_dump_" + "0" * 32}, "PAPER_BACKUP_FOREIGN_CLIENT"),
                 "early": ({"start_us": 3_999_999}, "PAPER_BACKUP_FOREIGN_CLIENT")}
        for name, (change, code) in cases.items():
            with self.subTest(name):
                owner, dump, exporter, session = self.owner()
                with self.assertRaises(backup.PaperBackupError) as raised:
                    owner._match_clients([exporter, dict(session, **change)])
                self.assertEqual(raised.exception.args, (code,))
                self.assertIsNone(dump.session)

    def test_two_sessions_a_changed_identity_and_any_session_after_close_are_refused(self):
        owner, dump, exporter, session = self.owner()
        with self.assertRaises(backup.PaperBackupError) as raised:
            owner._match_clients([exporter, session, dict(session, pid=12)])
        self.assertEqual(raised.exception.args, ("PAPER_BACKUP_FOREIGN_CLIENT",))
        owner._match_clients([exporter, session])
        for change in ({"pid": 12}, {"start_us": 4_000_001}):
            with self.subTest(change=change), self.assertRaises(backup.PaperBackupError) as raised:
                owner._match_clients([exporter, dict(session, **change)])
            self.assertEqual(raised.exception.args, ("PAPER_BACKUP_DUMP_SESSION",))
        dump.closed = True
        for row in (session, dict(session, pid=12)):
            with self.subTest(closed=row["pid"]), self.assertRaises(backup.PaperBackupError) as raised:
                owner._match_clients([exporter, row])
            self.assertEqual(raised.exception.args, ("PAPER_BACKUP_FOREIGN_CLIENT",))

    def test_application_name_never_stands_in_for_a_captured_client(self):
        owner, _dump, exporter, session = self.owner()
        with self.assertRaises(backup.PaperBackupError) as raised:
            owner._match_clients([session])
        self.assertEqual(raised.exception.args, ("PAPER_BACKUP_CLIENT_LOST",))
        with self.assertRaises(backup.PaperBackupError) as raised:
            owner._match_clients([dict(exporter, port=30101, app=session["app"]), session])
        self.assertEqual(raised.exception.args, ("PAPER_BACKUP_CLIENT_LOST",))


class LocalDumpTests(unittest.TestCase):
    """Real _dump/_LocalDump with a real Python child standing in for docker exec.

    Docker, the database container's processes and the observer are fakes; no PostgreSQL.
    """
    SNAPSHOT = "00000003-0000001B-1"
    TIMEOUT = (11, "sha256:timeout")
    PG_DUMP = (22, "sha256:pg_dump")
    CGROUP = "/system.slice/docker-" + "a" * 64 + ".scope"
    # argv: head (hex), seconds to sleep after writing, exit status, repetitions of bytes 0..255.
    PROGRAM = ("import sys,time\nout=sys.stdout.buffer\nout.write(bytes.fromhex(sys.argv[1]))\n"
               "out.write(bytes(range(256))*int(sys.argv[4]));out.flush()\n"
               "time.sleep(float(sys.argv[2]))\nsys.exit(int(sys.argv[3]))\n")

    def setUp(self):
        import hashlib
        self.root = tempfile.TemporaryDirectory()
        self.directory = os.open(self.root.name, os.O_RDONLY | os.O_DIRECTORY)
        self.rows, self.inside, self.standins, self.argv = [], {}, [], []
        owner = object.__new__(backup._HeldPaperSnapshot)
        owner._closed = owner._poisoned = False
        owner.maximum, owner.spools, owner.clients = 1 << 20, {}, []
        owner.package = None
        owner._snapshot = self.SNAPSHOT
        owner.root_tree = types.SimpleNamespace(fd=self.directory)
        owner.context = types.SimpleNamespace(database_generation=("a" * 64,), database={"database": "lagrange"})
        identity = lambda v: (v.st_dev, v.st_ino, v.st_mode, v.st_uid, v.st_gid, v.st_nlink, v.st_size, v.st_mtime_ns)
        owner.selection = types.SimpleNamespace(_backup_module=lambda: types.SimpleNamespace(
            files=types.SimpleNamespace(identity=identity)))
        started = backup.time.monotonic()
        def remaining(cap=15):
            left = 30 - (backup.time.monotonic() - started)
            if left <= 0:
                raise backup.PaperBackupError("PAPER_BACKUP_TEST_DEADLINE")
            return min(cap, left)
        observer = types.SimpleNamespace(
            database_principal=lambda: ({"Id": "a" * 64, "State": {"Pid": 4242}}, "lagrange"),
            database=lambda action: {"backends": [r for r in (row() for row in self.rows) if r is not None]})
        owner._custody = types.SimpleNamespace(observer=observer, revalidate=mock.Mock(),
            deadline=types.SimpleNamespace(remaining=remaining, work_remaining=remaining))
        self.main = {"pid": 4242, "start_ticks": 1, "parent": os.getpid(), "cgroup": self.CGROUP,
                     "executable": {"bytes": 33, "sha256": "sha256:postgres"}}
        def record(pid):
            if pid == 4242:
                return copy.deepcopy(self.main)
            stat = backup._proc_stat(pid)
            if stat is None:
                raise FileNotFoundError(pid)
            elf = self.inside.get(pid, (44, "sha256:docker"))
            return {"pid": pid, "start_ticks": stat[1], "parent": stat[0],
                    "cgroup": self.CGROUP if pid in self.inside else "/user.slice",
                    "executable": {"bytes": elf[0], "sha256": elf[1]}}
        def hashed(fd):
            raw = os.pread(fd, owner.maximum + 1, 0)
            return len(raw), "sha256:" + hashlib.sha256(raw).hexdigest()
        owner.ops = types.SimpleNamespace(PaperOperationsError=type("Fake", (Exception,), {}),
            _process_record=record, _hash_fd=hashed,
            _proc_cgroups=lambda: {4242: self.CGROUP, **{pid: self.CGROUP for pid in self.inside}},
            _docker_client_environment=lambda: {"PATH": "/usr/bin:/bin"})
        owner.exporter = backup._Client(owner, "exporter")
        owner.exporter.executables = {"/bin/sh": (9, "sha256:sh"), "/usr/bin/timeout": self.TIMEOUT,
                                      "/usr/lib/postgresql/18/bin/pg_dump": self.PG_DUMP}
        self.owner = owner

    def tearDown(self):
        dump = self.owner.local_dump
        if dump is not None:
            for fd in dump.pidfds.values():
                os.close(fd)
            if dump.child is not None:
                if dump.child.poll() is None:
                    dump.child.kill()
                dump.child.wait(timeout=5)
                dump.child.stdout.close()
        for process in self.standins:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=5)
        for fd, *_rest in self.owner.spools.values():
            os.close(fd)
        os.close(self.directory)
        self.root.cleanup()

    def dump(self, payload, status, *, sleep=0.0, standin=False, repeat=0, standin_sleep=30, settle=0.3):
        real = backup.subprocess.Popen
        def popen(argv, **kwargs):
            self.argv.append(argv)
            if standin:  # an in-container timeout of this attempt, started after the exec
                process = real([sys.executable, "-I", "-S", "-B", "-c",
                                "import time; time.sleep(%r)" % standin_sleep])
                self.standins.append(process)
                self.inside[process.pid] = self.TIMEOUT
            return real([sys.executable, "-I", "-S", "-B", "-c", self.PROGRAM, payload.hex(), str(sleep),
                         str(status), str(repeat)], **kwargs)
        with mock.patch.object(backup.subprocess, "Popen", side_effect=popen), \
                mock.patch.object(backup._LocalDump, "SETTLE", settle):
            self.owner._dump()

    def test_periodic_tick_binds_process_owns_client_and_admits_session(self):
        # The one-second tick runs the real check/owns/admit paths against a live exec.
        owner, ticks, session = self.owner, [], {}
        def served():
            dump = owner.local_dump
            if dump is None or dump.child is None or dump.child.poll() is not None:
                return None
            if not session:
                session.update(pid=77, start_us=backup.time.time_ns() // 1000, role="lagrange",
                               addr=None, port=-1, app=dump.application_name)
            return dict(session)
        self.rows.append(served)
        def tick():
            dump = owner.local_dump
            if dump is not None and not dump.closed:
                dump.check()
                ticks.append(owner._owns_process(dump.child.pid))
            owner._match_clients(owner._custody.observer.database("observe")["backends"])
        owner._custody.revalidate.side_effect = tick
        self.dump(b"PGDMPlive", 0, sleep=1.3, standin=True, standin_sleep=1.6, settle=2)
        dump, standin = owner.local_dump, self.standins[0]
        self.assertTrue(ticks and all(ticks))  # owned while alive and unchanged
        self.assertEqual(dump.session, session)
        self.assertEqual([kind for kind, _row in dump.processes.values()], ["timeout"])
        self.assertIn(standin.pid, dump.processes)
        self.assertTrue(dump.closed and dump.settled)
        self.assertFalse(owner._owns_process(dump.child.pid))
        self.assertEqual((Path(self.root.name) / "postgres.dump").read_bytes(), b"PGDMPlive")
        with self.assertRaises(backup.PaperBackupError) as raised:
            owner._match_clients([dict(session)])
        self.assertEqual(raised.exception.args, ("PAPER_BACKUP_FOREIGN_CLIENT",))

    def test_exact_local_argv_is_handed_to_popen(self):
        self.owner.exporter.exchange = mock.Mock()
        with mock.patch.object(backup.subprocess, "Popen", side_effect=OSError) as popen, \
                self.assertRaises(OSError):
            self.owner._dump()
        self.owner.exporter.exchange.assert_not_called()  # nothing runs through the capture client
        dump = self.owner.local_dump
        popen.assert_called_once()
        self.assertEqual(popen.call_args.args[0], backup.paper_dump_argv(
            "a" * 64, "lagrange", "lagrange", self.SNAPSHOT, dump.application_name))
        self.assertTrue(backup._DUMP_APPLICATION.fullmatch(dump.application_name))
        self.assertEqual(popen.call_args.kwargs["stdin"], backup.subprocess.DEVNULL)
        self.assertEqual(popen.call_args.kwargs["stderr"], backup.subprocess.DEVNULL)
        self.assertTrue(dump.closed)  # a spawn that never happened leaves nothing to signal

    def test_status_zero_pgdmp_archive_is_sealed(self):
        payload = b"PGDMP" + bytes(range(256)) * 600  # several pipe reads
        self.dump(b"PGDMP", 0, repeat=600)
        self.assertEqual(self.argv, [backup.paper_dump_argv(
            "a" * 64, "lagrange", "lagrange", self.SNAPSHOT, self.owner.local_dump.application_name)])
        self.assertEqual((Path(self.root.name) / "postgres.dump").read_bytes(), payload)
        self.assertEqual(self.owner.spools["postgres.dump"][2], len(payload))
        self.assertTrue(self.owner.local_dump.closed and self.owner.local_dump.settled)
        self.assertEqual(self.owner.local_dump.child.returncode, 0)
        self.assertFalse(self.owner._poisoned)

    def test_failed_short_or_foreign_archive_is_refused(self):
        for name, payload, status in (("status", b"PGDMPcomplete", 1), ("short", b"PGDM", 0),
                                      ("magic", b"PGDMXcomplete", 0)):
            with self.subTest(name):
                self.tearDown(); self.setUp()
                with self.assertRaises(backup.PaperBackupError) as raised:
                    self.dump(payload, status)
                self.assertEqual(raised.exception.args, ("PAPER_BACKUP_DUMP_FAILED",))
                self.assertNotIn("postgres.dump", self.owner.spools)
                self.assertTrue(self.owner.local_dump.closed)

    def test_limit_is_checked_before_write_and_the_child_is_reaped(self):
        self.owner.maximum = 64
        with self.assertRaises(backup.PaperBackupError) as raised:
            self.dump(b"PGDMP" + b"x" * 100, 0, sleep=30)
        self.assertEqual(raised.exception.args, ("PAPER_BACKUP_LIMIT",))
        self.assertEqual((Path(self.root.name) / "postgres.dump").read_bytes(), b"")
        dump = self.owner.local_dump
        self.assertEqual(dump.child.returncode, -backup.signal.SIGTERM)
        self.assertTrue(dump.closed)

    def test_session_left_after_exit_is_cleanup_unknown_and_retained(self):
        self.rows.append(lambda: {"pid": 77, "start_us": backup.time.time_ns() // 1000, "role": "lagrange",
                                  "addr": None, "port": -1, "app": self.owner.local_dump.application_name})
        with self.assertRaises(backup.PaperBackupError) as raised:
            self.dump(b"PGDMPcomplete", 0)
        self.assertEqual(raised.exception.args, ("PAPER_BACKUP_CLEANUP_UNKNOWN",))
        self.assertNotIn("postgres.dump", self.owner.spools)
        self.assertFalse(self.owner.local_dump.closed)
        self.assertTrue(self.owner._poisoned)
        self.assertFalse(self.owner._resources_closed())

    def test_in_container_process_left_after_exit_is_cleanup_unknown_and_only_it_is_signalled(self):
        with self.assertRaises(backup.PaperBackupError) as raised:
            self.dump(b"PGDMPcomplete", 0, standin=True)
        self.assertEqual(raised.exception.args, ("PAPER_BACKUP_CLEANUP_UNKNOWN",))
        self.assertNotIn("postgres.dump", self.owner.spools)
        dump, standin = self.owner.local_dump, self.standins[0]
        self.assertEqual(dump.processes[standin.pid][0], "timeout")
        # The cancellation path signalled only the process it had bound, by pidfd.
        self.assertEqual(standin.wait(timeout=5), -backup.signal.SIGKILL)
        self.assertTrue(dump.closed)

    def test_process_started_before_the_exec_or_with_another_elf_is_never_bound(self):
        early = backup.subprocess.Popen([sys.executable, "-I", "-S", "-B", "-c", "import time; time.sleep(30)"])
        self.standins.append(early)
        self.inside[early.pid] = self.TIMEOUT
        backup.time.sleep(0.05)
        real = backup.subprocess.Popen
        def popen(argv, **kwargs):
            late = real([sys.executable, "-I", "-S", "-B", "-c", "import time; time.sleep(30)"])
            self.standins.append(late)
            self.inside[late.pid] = (55, "sha256:pg_isready")
            return real([sys.executable, "-I", "-S", "-B", "-c", self.PROGRAM, b"PGDMPok".hex(), "0", "0", "0"],
                        **kwargs)
        with mock.patch.object(backup.subprocess, "Popen", side_effect=popen), \
                mock.patch.object(backup._LocalDump, "SETTLE", 0.3):
            self.owner._dump()
        self.assertEqual(self.owner.local_dump.processes, {})
        self.assertTrue(all(process.poll() is None for process in self.standins))


class ClientPolicyTests(unittest.TestCase):
    """Run the real _Client/_GuestClient policy against fake Docker inspect records."""
    NETWORK_ID = "n" * 64
    IMAGE_ID = "sha256:" + "i" * 64
    CODE = "PAPER_BACKUP_CLIENT_CHANGED"
    GUEST_CODE = "PAPER_BACKUP_GUEST_CLIENT_CHANGED"
    VOLUME = {"Type": "volume", "Destination": "/var/lib/postgresql", "Driver": "local",
              "RW": True, "Name": "a" * 64}

    @staticmethod
    def environment(value):
        return dict(item.split("=", 1) for item in value["Config"]["Env"])

    NETWORK = {"Id": NETWORK_ID, "Name": "db-net"}

    def fake_owner(self, calls, network=NETWORK):
        images = {self.IMAGE_ID: {"Config": {"Env": ["PATH=/usr/bin"]}}}

        def lookup(kind, identity):
            calls.append((kind, identity))
            if kind == "image":
                return images[identity]
            self.assertEqual(kind, "network")
            self.assertEqual(identity, self.NETWORK_ID)
            return network
        docker = mock.Mock()
        docker.object.side_effect = lookup
        owner = mock.Mock()
        owner.context = types.SimpleNamespace(
            docker=docker, database_image_id=self.IMAGE_ID, network_id=self.NETWORK_ID)
        owner.ops.runtime.environment = self.environment
        return owner

    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(lambda: __import__("shutil").rmtree(self.tmp, ignore_errors=True))
        self.tmp = os.path.realpath(self.tmp)
        self.calls, self.guest_calls = [], []
        self.owner = self.fake_owner(self.calls)
        self.owner.root = Path(self.tmp)
        self.client = object.__new__(backup._Client)
        self.client.owner, self.client.cid, self.client.created = self.owner, "c" * 64, "created-at"
        self.client.name, self.client.command = "lagrange-paper-capture-x", ["-X", "-qAt"]
        self.client.kind = "capture"
        self.guest = object.__new__(backup._GuestClient)
        self.guest_owner = self.fake_owner(self.guest_calls)
        real_ops = backup._local("paper_client_policy_test_ops", "owner-beta-paper-operations.py")
        self.guest_owner.ops = types.SimpleNamespace(
            _validate_mount_records=real_ops._validate_mount_records,
            runtime=types.SimpleNamespace(environment=self.environment))
        self.ops_error = real_ops.PaperOperationsError
        package = types.SimpleNamespace(guest=types.SimpleNamespace(
            check=mock.Mock(), image=self.IMAGE_ID, socket=Path(self.tmp) / "sock"))
        self.guest.owner, self.guest.package = self.guest_owner, package
        self.guest.cid, self.guest.created = "g" * 64, "guest-created-at"
        self.guest.name, self.guest.command = "lagrange-paper-capture-g", ["-X", "-qAt"]
        self.guest.environment = {"PGPASSFILE": "/dev/null", "PGSERVICEFILE": "/dev/null",
            "PGCONNECT_TIMEOUT": "10", "PGCLIENTENCODING": "UTF8",
            "PGOPTIONS": "-c default_transaction_read_only=on -c statement_timeout=900000 -c lock_timeout=5000"}

    def common(self, cid, created, name, command, env):
        return {"Id": cid, "Created": created, "Image": self.IMAGE_ID, "RestartCount": 0,
            "Config": {"Image": self.IMAGE_ID, "User": "999:999",
                       "Entrypoint": ["/usr/lib/postgresql/18/bin/psql"], "Cmd": list(command),
                       "Labels": {"io.lagrange.paper.capture": name},
                       "Env": ["PATH=/usr/bin"] + [k + "=" + v for k, v in env.items()]},
            "State": {"Status": "created", "OOMKilled": False}}

    def accepted(self):
        c = self.client
        env = {"PGPASSFILE": "/run/pgpass", "PGSERVICEFILE": "/dev/null", "PGCONNECT_TIMEOUT": "10",
               "PGCLIENTENCODING": "UTF8",
               "PGOPTIONS": "-c default_transaction_read_only=on -c statement_timeout=900000 -c lock_timeout=5000"}
        value = self.common(c.cid, c.created, c.name, c.command, env)
        value["HostConfig"] = {"ReadonlyRootfs": True, "Privileged": False,
            "RestartPolicy": {"Name": "no"}, "NetworkMode": self.NETWORK_ID,
            "NanoCpus": 1_000_000_000, "Memory": 536_870_912, "CapDrop": ["ALL"],
            "SecurityOpt": ["no-new-privileges"], "LogConfig": {"Type": "none", "Config": {}},
            "Tmpfs": {"/var/lib/postgresql": backup._VOLUME_COVER}}
        value["Mounts"] = [{"Type": "bind", "Source": str(Path(self.tmp) / "pgpass"),
                            "Destination": "/run/pgpass", "RW": False}]
        value["NetworkSettings"] = {"Networks": {"db-net": {"NetworkID": ""}}}
        return value

    def accepted_guest(self):
        g = self.guest
        value = self.common(g.cid, g.created, g.name, g.command, g.environment)
        value["HostConfig"] = {"NetworkMode": "none", "ReadonlyRootfs": True, "Privileged": False,
            "CapDrop": ["ALL"], "RestartPolicy": {"Name": "no"}, "NanoCpus": 1_000_000_000,
            "Memory": 536_870_912, "SecurityOpt": ["no-new-privileges"],
            "LogConfig": {"Type": "none"}, "PidMode": "", "IpcMode": "private",
            "Tmpfs": {"/var/lib/postgresql": backup._VOLUME_COVER}}
        value["Mounts"] = [{"Type": "bind", "Source": str(g.package.guest.socket),
                            "Destination": "/run/postgresql", "RW": False}]
        value["NetworkSettings"] = {"Networks": {"none": {"IPAddress": ""}}}
        return value

    def kinds(self):
        return [kind for kind, _ in self.calls]

    def reject(self, policy, value, code=None, error=backup.PaperBackupError):
        with self.assertRaises(error) as caught:
            policy(value)
        self.assertEqual(caught.exception.args, (code or self.CODE,))

    def test_capture_client_accepts_created_and_running_records(self):
        policy = self.client.policy
        value = self.accepted()
        policy(value)
        self.assertIn("network", self.kinds())
        del self.calls[:]
        value = self.accepted()
        value["State"]["Status"] = "running"
        value["NetworkSettings"]["Networks"] = {"db-net": {"NetworkID": self.NETWORK_ID}}
        policy(value)
        self.assertNotIn("network", self.kinds())
        value = self.accepted()
        value["NetworkSettings"]["Networks"] = {"db-net": {"NetworkID": self.NETWORK_ID}}
        policy(value)

    def test_capture_client_rejects_wrong_networks(self):
        cases = {
            "created pending other name": ("created", {"other": {"NetworkID": ""}}, None),
            "created pending other lookup id": ("created", {"db-net": {"NetworkID": ""}},
                                                {"Id": "z" * 64, "Name": "db-net"}),
            "running empty id": ("running", {"db-net": {"NetworkID": ""}}, None),
            "running other id": ("running", {"db-net": {"NetworkID": "y" * 64}}, None),
            "running two endpoints": ("running", {"db-net": {"NetworkID": self.NETWORK_ID},
                                                  "bridge": {"NetworkID": "x"}}, None),
            "created two endpoints": ("created", {"db-net": {"NetworkID": self.NETWORK_ID},
                                                  "bridge": {"NetworkID": "x"}}, None),
            "created two pending": ("created", {"db-net": {"NetworkID": ""},
                                                "bridge": {"NetworkID": ""}}, None),
            "running none": ("running", {}, None),
            "created none": ("created", {}, None),
        }
        for label, (status, networks, lookup) in cases.items():
            with self.subTest(label):
                value = self.accepted()
                value["State"]["Status"] = status
                value["NetworkSettings"]["Networks"] = networks
                self.owner = self.fake_owner(self.calls, lookup or self.NETWORK)
                self.owner.root = Path(self.tmp)
                self.client.owner = self.owner
                self.reject(self.client.policy, value)

    def test_capture_client_rejects_wrong_mounts_and_cover(self):
        def mutate(name):
            value = self.accepted()
            if name == "volume":
                value["Mounts"].append(dict(self.VOLUME))
            elif name == "no cover":
                del value["HostConfig"]["Tmpfs"]
            elif name == "guest cover":
                value["HostConfig"]["Tmpfs"] = {"/var/lib/postgresql": backup._GUEST_VOLUME_COVER}
            elif name == "writable":
                value["Mounts"][0]["RW"] = True
            elif name == "destination":
                value["Mounts"][0]["Destination"] = "/run/other"
            elif name == "source":
                value["Mounts"][0]["Source"] = str(Path(self.tmp) / "other")
            elif name == "second bind":
                value["Mounts"].append({"Type": "bind", "Source": str(Path(self.tmp) / "x"),
                                        "Destination": "/run/x", "RW": False})
            return value
        for name in ("volume", "no cover", "guest cover", "writable", "destination", "source", "second bind"):
            with self.subTest(name):
                self.reject(self.client.policy, mutate(name))
        value = self.accepted()
        value["Mounts"].append({"Type": "tmpfs", "Destination": "/var/lib/postgresql"})
        self.client.policy(value)

    def test_guest_client_accepts_its_record(self):
        self.guest.policy(self.accepted_guest())
        self.guest.package.guest.check.assert_called_once_with(ready=True)

    def test_guest_client_rejects_wrong_mounts_cover_and_network(self):
        value = self.accepted_guest()
        value["Mounts"].append(dict(self.VOLUME))
        self.reject(self.guest.policy, value, "PAPER_OPS_MOUNT", self.ops_error)
        value = self.accepted_guest()
        del value["HostConfig"]["Tmpfs"]
        self.reject(self.guest.policy, value, self.GUEST_CODE)
        value = self.accepted_guest()
        value["HostConfig"]["Tmpfs"] = {"/var/lib/postgresql": backup._GUEST_VOLUME_COVER}
        self.reject(self.guest.policy, value, self.GUEST_CODE)
        value = self.accepted_guest()
        value["NetworkSettings"]["Networks"] = {"db-net": {"NetworkID": "x", "IPAddress": "10.0.0.2"}}
        self.reject(self.guest.policy, value, self.GUEST_CODE)


class DatabaseImageVolumeCoverTests(unittest.TestCase):
    BIND = {"Type": "bind", "Source": "/x", "Destination": "/run/pgpass", "RW": False}
    CODE = "PAPER_BACKUP_CLIENT_CHANGED"

    def covered(self, mounts, cover=None):
        cover = backup._VOLUME_COVER if cover is None else cover
        return {"HostConfig": {"Tmpfs": {"/var/lib/postgresql": cover}}, "Mounts": mounts}

    def test_cover_is_required_exactly(self):
        bind, code = self.BIND, self.CODE
        cover = backup._VOLUME_COVER
        self.assertEqual(backup._covered_mounts(self.covered([bind]), cover, code), [bind])
        listed = {"Type": "tmpfs", "Destination": "/var/lib/postgresql"}
        self.assertEqual(backup._covered_mounts(self.covered([bind, listed]), cover, code), [bind])
        for mounts in ("missing", None):
            value = self.covered([bind])
            if mounts == "missing":
                del value["Mounts"]
            else:
                value["Mounts"] = None
            with self.subTest(mounts=mounts):
                self.assertEqual(backup._covered_mounts(value, cover, code), [])
        rejected = {
            "tmpfs missing": {"HostConfig": {}, "Mounts": [bind]},
            "tmpfs none": {"HostConfig": {"Tmpfs": None}, "Mounts": [bind]},
            "tmpfs empty": {"HostConfig": {"Tmpfs": {}}, "Mounts": [bind]},
            "guest cover": self.covered([bind], backup._GUEST_VOLUME_COVER),
            "extra tmpfs": {"HostConfig": {"Tmpfs": {"/var/lib/postgresql": cover, "/tmp": "rw"}},
                            "Mounts": [bind]},
            "host config missing": {"Mounts": [bind]},
            "mounts dict": self.covered({"Type": "bind"}),
        }
        for name, value in rejected.items():
            with self.subTest(name=name), self.assertRaises(backup.PaperBackupError) as caught:
                backup._covered_mounts(value, cover, code)
            self.assertEqual(caught.exception.args, (code,))

    def test_other_mounts_are_not_filtered(self):
        extras = [{"Type": "volume", "Destination": "/var/lib/postgresql", "Driver": "local",
                   "RW": True, "Name": "a" * 64},
                  {"Type": "tmpfs", "Destination": "/tmp"}, "x"]
        for extra in extras:
            with self.subTest(extra=extra):
                result = backup._covered_mounts(
                    self.covered([self.BIND, extra]), backup._VOLUME_COVER, self.CODE)
                self.assertEqual(result, [self.BIND, extra])

    def test_guest_cover_differs_only_in_mode(self):
        # Observed on a real Docker: with mode=000 the guest's data bind below the cover is
        # unreachable for uid 999.
        self.assertEqual(backup._IMAGE_VOLUME, "/var/lib/postgresql")
        self.assertEqual(backup._VOLUME_COVER, "rw,nosuid,nodev,noexec,size=4k,mode=000")
        self.assertEqual(backup._GUEST_VOLUME_COVER, "rw,nosuid,nodev,noexec,size=4k,mode=0711")

    def test_every_database_image_container_is_created_with_the_cover(self):
        source = PATH.read_text()
        for fragment in (
                '"--tmpfs", _IMAGE_VOLUME + ":" + _GUEST_VOLUME_COVER, "--entrypoint", "/bin/bash"]',
                '"--tmpfs", _IMAGE_VOLUME + ":" + _VOLUME_COVER,\n'
                '            "--mount", "type=bind,source=" + str(o.root / "pgpass")',
                'args.extend(("--tmpfs", _IMAGE_VOLUME + ":" + _VOLUME_COVER,'):
            with self.subTest(fragment=fragment):
                self.assertEqual(source.count(fragment), 1)
        self.assertNotIn('not host.get("Tmpfs")', source)


if __name__ == "__main__":
    unittest.main()
