#!/usr/bin/env python3
"""Pure resource decisions only. These mocks never mint hosted authority."""
import copy
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
        guest.create_sent, guest.cid, guest.absent = False, None, False
        with mock.patch.object(backup, "_catalog_restore_sql", side_effect=backup.PaperBackupError("PAPER_BACKUP_CATALOG")):
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
                LogConfig={"Type": "none"}, SecurityOpt=["no-new-privileges:true"], PidMode="", IpcMode="private"),
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
            audit_module=types.SimpleNamespace(decode=mock.Mock(), parse=json.loads),
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
            audit_module=types.SimpleNamespace(decode=mock.Mock(), parse=json.loads))
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
        client = types.SimpleNamespace(backend=row, absent=False, dumping=False)
        owner.clients = [client]
        return owner, client, row

    def test_actual_backend_set_cannot_be_waived_by_role_or_application(self):
        owner, client, row = self.owner()
        owner._match_clients([dict(row)])
        for rows in ([], [dict(row, start_us=21)], [dict(row), dict(row, pid=11)],
                     [dict(row, role="app")], [dict(row, port=30101)]):
            with self.subTest(rows=len(rows)), self.assertRaises(backup.PaperBackupError):
                owner._match_clients(rows)

    def test_dump_client_requires_exact_owned_socket_and_generation(self):
        owner, client, row = self.owner()
        client.dumping = True
        client.executables = {"/usr/lib/postgresql/18/bin/pg_dump": (7, "hash")}
        process = {"pid": 44, "start_ticks": 55, "executable": {"bytes": 7, "sha256": "hash"}}
        client.child_processes = lambda: {44: process}
        client.dump_backends = {}
        socket = {"local": ["10.0.0.3", 30101], "peer": ["10.0.0.1", 5432]}
        owner.ops._tcp_endpoints.return_value = [socket]
        dump = dict(row, pid=11, start_us=22, addr="10.0.0.3", port=30101)
        owner._match_clients([row, dump])
        self.assertEqual(client.dump_backends[11][0], dump)
        for change in ({"start_us": 23}, {"addr": "10.0.0.4"}, {"role": "app"}):
            with self.assertRaises(backup.PaperBackupError):
                owner._match_clients([row, dict(dump, **change)])
        client.dumping = False
        with self.assertRaises(backup.PaperBackupError):
            owner._match_clients([row, dump])

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

    def test_dump_uses_original_opaque_export_and_requires_actual_terminal_status(self):
        for exit_code in (0, 1):
            with self.subTest(exit_code=exit_code), tempfile.TemporaryDirectory() as root:
                owner, client, row = self.owner()
                owner.root = Path(root)
                owner._new_spool = lambda name: open(owner.root / name, "x+b")
                owner._seal_spool = mock.Mock()
                owner._snapshot = "opaque'identifier"
                owner.exporter = client
                client.dumping = False
                client.dump_backends = {11: (dict(row, pid=11, start_us=22), None, None)}
                client.child_processes = lambda: {}
                owner._custody = types.SimpleNamespace(observer=types.SimpleNamespace(
                    database=lambda action: {"backends": [row]}))
                commands = []
                def exchange(sql, stream):
                    commands.append(sql)
                    self.assertIn(b"--snapshot='opaque'", sql)
                    self.assertIn(b"900s", sql)
                    self.assertTrue(sql.startswith(b"\\! "))
                    status = backup.re.search(rb"PAPER_DUMP_[a-f0-9]{32}", sql)[0]
                    stream.write(b"PGDMPbinarydump\n" + status + b"_" + str(exit_code).encode() + b"\n")
                client.exchange = exchange
                with mock.patch.object(backup._HeldPaperSnapshot, "revalidate"):
                    if exit_code:
                        with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_DUMP_FAILED"):
                            owner._dump()
                    else:
                        owner._dump()
                        self.assertEqual((owner.root / "postgres.dump").read_bytes(), b"PGDMPbinarydump")
                self.assertEqual(len(commands), 1)
                self.assertFalse(client.dumping)

    def test_dump_requires_captured_backends_absent_after_shell_completion(self):
        owner, client, row = self.owner()
        owner.exporter = client
        owner._snapshot = "opaque"
        client.dump_backends = {11: (dict(row, pid=11, start_us=22), None, None)}
        client.child_processes = lambda: {}
        owner._custody = types.SimpleNamespace(observer=types.SimpleNamespace(
            database=lambda action: {"backends": [row, dict(row, pid=11, start_us=22)]}))
        def exchange(sql, stream):
            status = backup.re.search(rb"PAPER_DUMP_[a-f0-9]{32}", sql)[0]
            stream.write(b"PGDMPbinarydump\n" + status + b"_0\n")
        client.exchange = exchange
        with tempfile.TemporaryDirectory() as root, mock.patch.object(backup._HeldPaperSnapshot, "revalidate"):
            owner.root = Path(root)
            owner._new_spool = lambda name: open(owner.root / name, "x+b")
            owner._seal_spool = mock.Mock()
            with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_CLEANUP_UNKNOWN"):
                owner._dump()

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
        refs = types.SimpleNamespace(prepare_references=mock.Mock(return_value=pending))
        owner.selection = types.SimpleNamespace(open_selected_paper_files=mock.Mock(return_value=selected))
        owner.audit = io.BytesIO(b"component-stream")
        with tempfile.TemporaryDirectory() as root, \
                mock.patch.object(backup._HeldPaperSnapshot, "revalidate"), \
                mock.patch.object(backup._HeldPaperSnapshot, "_dump") as dump, \
                mock.patch.object(backup._HeldPaperSnapshot, "_archive_capture") as archive, \
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
            self.assertEqual(sorted(p.name for p in owner.root.iterdir()), ["files.bundle"])
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

    def test_unobserved_dump_backend_cannot_be_inferred_from_successful_exit(self):
        owner, client, row = self.owner()
        owner.exporter, owner._snapshot = client, "opaque"
        client.dump_backends = {}
        client.child_processes = lambda: {}
        def exchange(sql, stream):
            status = backup.re.search(rb"PAPER_DUMP_[a-f0-9]{32}", sql)[0]
            stream.write(b"PGDMPbinarydump\n" + status + b"_0\n")
        client.exchange = exchange
        with tempfile.TemporaryDirectory() as root, mock.patch.object(backup._HeldPaperSnapshot, "revalidate"):
            owner.root = Path(root)
            owner._new_spool = lambda name: open(owner.root / name, "x+b")
            owner._seal_spool = mock.Mock()
            with self.assertRaisesRegex(backup.PaperBackupError, "PAPER_BACKUP_DUMP_BACKEND_UNOBSERVED"):
                owner._dump()

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
                                               (object(), True, "PAPER_BACKUP_REFERENCE_CLOSURE_INCOMPLETE")):
            with self.subTest(configured=configured):
                inventory = types.SimpleNamespace(_presence=presence, _configured=configured,
                    revalidate=mock.Mock(), _pages=[(None,b"page")], _local=types.SimpleNamespace(operations=[]))
                live = types.SimpleNamespace(probe_history=lambda: __import__("contextlib").nullcontext(inventory), close=mock.Mock())
                ops = types.SimpleNamespace(_HISTORY_EMPTY=empty, MAX_PAGE=4096,
                    _json=lambda *args: {"account_id": "account", "control_epoch": 0}, open_context=lambda *args: live)
                with mock.patch.object(backup, "_application_context", return_value=(ops, object())):
                    if expected is None:
                        self.assertIsNone(backup.run_locked(object(), 3, "verify", 10))
                    else:
                        with self.assertRaisesRegex(backup.PaperBackupError, expected):
                            backup.run_locked(object(), 3, "verify", 10)
                live.close.assert_called_once()

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

    def test_fast_dump_exchange_arms_and_observes_before_accepting_completion(self):
        # A complete reply in the FIRST pipe read precedes the periodic tick.
        # This kernel/pipe model has no wall-clock wait or PostgreSQL authority.
        events = []
        owner = types.SimpleNamespace(maximum=10000, revalidate=mock.Mock(),
            _custody=types.SimpleNamespace(deadline=types.SimpleNamespace(work_remaining=lambda cap=15: cap)))
        client = backup._Client(owner, "exporter")
        client.dumping, client.backend = True, {"pid": 10}
        client.child = types.SimpleNamespace(poll=lambda: None,
            stdin=types.SimpleNamespace(fileno=lambda: 3), stdout=types.SimpleNamespace(fileno=lambda: 4))
        class Gate:
            closed = False
            def __init__(self, actual):
                self.actual = actual
                actual.dump_gate = self
                events.append("arm")
            def pump(self):
                events.append("held-observation")
                self.actual.dump_backends[11] = ("component-only",)
                self.closed = True
            def close(self):
                events.append("detach")
                self.closed = True
        written = []
        def write(fd, data):
            events.append("write")
            written.append(bytes(data))
            return len(data)
        def read(fd, size):
            marker = written[0].split(b"\\echo ")[1].splitlines()[0]
            return b"PGDMPshort\n" + marker + b"\n"
        with mock.patch.object(backup, "_DumpGate", Gate, create=True), \
                mock.patch.object(backup.os, "set_blocking"), \
                mock.patch.object(backup.os, "write", side_effect=write), \
                mock.patch.object(backup.os, "read", side_effect=read), \
                mock.patch.object(backup.select, "select", return_value=([4], [3], [])), \
                mock.patch.object(backup.time, "monotonic", return_value=0):
            self.assertEqual(client.exchange(b"\\! actual-source-command", io.BytesIO()), 11)
        self.assertTrue(client.dump_backends)
        self.assertLess(events.index("arm"), events.index("write"))
        self.assertIn("held-observation", events)
        self.assertEqual(events[-1], "detach")
        owner.revalidate.assert_not_called()  # the one-second tick never occurred

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
                    self.assertEqual(os.stat(owner.root / "audit.stream").st_mode & 0o777, 0o600)
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


class DumpGateTests(unittest.TestCase):
    def gate(self):
        owner, _unused, exporter = PaperCaptureTests().owner()
        client = backup._Client(owner, "exporter")
        client.dumping, client.backend = True, exporter
        parent = {"pid": 40, "start_ticks": 50, "parent": 30, "cgroup": "/owned",
                  "executable": {"bytes": 8, "sha256": "psql", "device": 1, "inode": 2}}
        process = {"pid": 44, "start_ticks": 55, "parent": 40, "cgroup": "/owned",
                   "executable": {"bytes": 7, "sha256": "dump", "device": 1, "inode": 3}}
        endpoint = {"local": ["10.0.0.3", 30101], "peer": ["10.0.0.1", 5432],
                    "socket_inode": 123, "pid": 44, "start_ticks": 55}
        backend = dict(exporter, pid=11, start_us=22, addr="10.0.0.3", port=30101)
        client.process = parent
        client.executables = {"/usr/lib/postgresql/18/bin/pg_dump": (7, "dump"), "/bin/sh": (9, "sh")}
        client.child_processes = lambda: {44: process}
        owner.clients = [client]
        owner.ops._tcp_endpoints.return_value = [endpoint]
        owner.ops._process_record = mock.Mock(side_effect=lambda pid: copy.deepcopy({40: parent, 44: process}[pid]))
        owner._custody = types.SimpleNamespace(deadline=types.SimpleNamespace(
            work_remaining=mock.Mock(return_value=1), remaining=mock.Mock(return_value=1)))
        owner.revalidate = mock.Mock(side_effect=lambda: owner._match_clients([exporter, backend]))
        gate = object.__new__(backup._DumpGate)
        gate.client, gate.owner = client, owner
        gate.rows, gate.stopped = {40: copy.deepcopy(parent), 44: copy.deepcopy(process)}, {44}
        gate.pending, gate.signals, gate.syscall_stops = set(), {}, set()
        gate.group_stopped = {}
        gate.dump_pid, gate.closed = 44, False
        client.dump_gate = gate
        return gate, client, owner, process, endpoint, backend

    def test_socket_stop_uses_actual_full_observer_then_repeats_before_detach(self):
        gate, client, owner, process, endpoint, backend = self.gate()
        order = []
        original = owner.revalidate.side_effect
        owner.revalidate.side_effect = lambda: (order.append("observe-held"), original())
        with mock.patch.object(backup.os, "readlink", return_value="socket:[123]"), \
                mock.patch.object(gate, "_stop_all", side_effect=lambda cleanup: order.append("hold")), \
                mock.patch.object(gate, "close", side_effect=lambda: order.append("detach")):
            gate._socket_boundary(44, 6, terminal=False)
            self.assertEqual(client.dump_backends, {11: (backend, process, endpoint)})
            gate._release_or_capture()
        self.assertEqual(order, ["hold", "observe-held", "hold", "observe-held", "detach"])

    def test_seize_group_stop_retains_custody_without_resuming_or_detaching(self):
        for signum in (backup.signal.SIGSTOP, backup.signal.SIGTSTP,
                       backup.signal.SIGTTIN, backup.signal.SIGTTOU):
            with self.subTest(signal=signum):
                gate, client, _owner, process, _endpoint, _backend = self.gate()
                status = (128 << 16) | (signum << 8) | 0x7f
                with mock.patch.object(gate, "_ptrace") as ptrace:
                    with self.assertRaisesRegex(backup.PaperBackupError, "DUMP_GATE_SIGNAL"):
                        gate._event(44, status)
                    self.assertEqual(gate.group_stopped, {44: signum})
                    self.assertEqual(gate.rows[44], process)
                    self.assertIn(44, gate.stopped)
                    with self.assertRaisesRegex(backup.PaperBackupError, "DUMP_GATE_SIGNAL"):
                        gate._resume(44)
                    with self.assertRaisesRegex(backup.PaperBackupError, "DUMP_GATE_SIGNAL"):
                        gate.close()
                    ptrace.assert_not_called()
                self.assertIs(client.dump_gate, gate)
                self.assertFalse(gate.closed)

    def test_seize_interrupt_trap_is_distinct_from_external_group_stop(self):
        gate, _client, _owner, _process, _endpoint, _backend = self.gate()
        status = (128 << 16) | (backup.signal.SIGTRAP << 8) | 0x7f
        gate._event(44, status)
        self.assertEqual(gate.group_stopped, {})
        with mock.patch.object(gate, "_ptrace") as ptrace:
            gate._resume(44)
            ptrace.assert_called_once_with(gate.SYSCALL, 44, 0)

    def test_unknown_seize_stop_is_rejected_and_not_detached(self):
        gate, _client, _owner, _process, _endpoint, _backend = self.gate()
        status = (128 << 16) | (backup.signal.SIGUSR1 << 8) | 0x7f
        with self.assertRaisesRegex(backup.PaperBackupError, "DUMP_GATE_SIGNAL"):
            gate._event(44, status)
        with mock.patch.object(gate, "_ptrace") as ptrace:
            with self.assertRaisesRegex(backup.PaperBackupError, "DUMP_GATE_SIGNAL"):
                gate.close()
            ptrace.assert_not_called()

    def test_missing_backend_is_never_exit_or_close_authority(self):
        gate, client, owner, _p, _e, _b = self.gate()
        owner.revalidate.side_effect = lambda: owner._match_clients([client.backend])
        with mock.patch.object(backup.os, "readlink", return_value="socket:[123]"), \
                mock.patch.object(gate, "_stop_all"):
            gate._socket_boundary(44, 6, terminal=False)  # startup can proceed
            self.assertEqual(client.dump_backends, {})
            with self.assertRaisesRegex(backup.PaperBackupError, "DUMP_BACKEND_UNOBSERVED"):
                gate._socket_boundary(44, 6, terminal=True)
        self.assertFalse(gate.closed)

    def test_foreign_reused_or_changed_backend_is_rejected_while_held(self):
        for change in ({"role": "app"}, {"addr": "10.0.0.9"}, {"port": 30102}, {"start_us": 23}):
            with self.subTest(change=change):
                gate, client, owner, _p, _e, backend = self.gate()
                with mock.patch.object(backup.os, "readlink", return_value="socket:[123]"), \
                        mock.patch.object(gate, "_stop_all"):
                    if "start_us" in change:
                        gate._socket_boundary(44, 6, terminal=False)
                    owner.revalidate.side_effect = lambda: owner._match_clients([client.backend, dict(backend, **change)])
                    with self.assertRaises(backup.PaperBackupError):
                        gate._socket_boundary(44, 6, terminal=False)
                self.assertFalse(gate.closed)

    def test_pid_executable_and_socket_replacement_never_resume(self):
        for change in ({"start_ticks": 56}, {"cgroup": "/foreign"}, {"executable": {"bytes": 7, "sha256": "foreign"}}):
            gate, _c, owner, process, _e, _b = self.gate()
            owner.ops._process_record.return_value = dict(process, **change)
            owner.ops._process_record.side_effect = None
            with self.assertRaisesRegex(backup.PaperBackupError, "CLIENT_CHANGED"):
                gate._identity(44)
        gate, _c, _o, _p, _e, _b = self.gate()
        with mock.patch.object(backup.os, "readlink", side_effect=["socket:[123]", "socket:[124]"]), \
                mock.patch.object(gate, "_stop_all"), \
                self.assertRaisesRegex(backup.PaperBackupError, "DUMP_GATE_SOCKET"):
            gate._socket_boundary(44, 6, terminal=False)

    def test_missing_fd_or_missing_kernel_wait_custody_fails(self):
        gate, _c, _o, _p, _e, _b = self.gate()
        with mock.patch.object(backup.os, "readlink", side_effect=FileNotFoundError), \
                self.assertRaisesRegex(backup.PaperBackupError, "DUMP_GATE_SOCKET"):
            gate._socket_boundary(44, 6, terminal=False)
        with mock.patch.object(backup.os, "waitpid", side_effect=ChildProcessError), \
                self.assertRaisesRegex(backup.PaperBackupError, "DUMP_GATE_LOST"):
            gate._poll()
        self.assertIn(40, gate.rows)

    def syscall(self, gate, phase, number=44, arch=0xC000003E):
        raw = bytearray(80)
        raw[0] = phase
        backup.struct.pack_into("<I", raw, 4, arch)
        backup.struct.pack_into("<QQ", raw, 24, number, 6)
        def ptrace(request, pid, data=0, address=0):
            self.assertEqual((request, pid, address), (gate.GETINFO, 44, 128))
            backup.ctypes.memmove(data, bytes(raw), len(raw))
            return len(raw)
        return mock.patch.object(gate, "_ptrace", side_effect=ptrace)

    def test_kernel_syscall_info_not_phase_guess_controls_network_gate(self):
        gate, _c, _o, _p, _e, _b = self.gate()
        for number in gate.SOCKET_CALLS:
            with self.syscall(gate, 1, number), mock.patch.object(gate, "_socket_boundary") as boundary:
                gate._syscall(44)
                boundary.assert_called_once_with(44, 6, terminal=number in (3, 48))
        for _ in range(2):  # repeated EXIT is not toggled into a fake ENTRY
            with self.syscall(gate, 2), mock.patch.object(gate, "_socket_boundary") as boundary:
                gate._syscall(44)
                boundary.assert_not_called()
        for phase, number, arch in ((0,44,0xC000003E), (1,44,0), (1,60,0xC000003E),
                                     (1,231,0xC000003E), (1,425,0xC000003E), (1,436,0xC000003E)):
            with self.syscall(gate, phase, number, arch), self.assertRaises(backup.PaperBackupError):
                gate._syscall(44)

    def test_exec_requires_same_generation_and_actual_pinned_dump_elf(self):
        gate, _c, owner, process, _e, _b = self.gate()
        gate.dump_pid = None
        gate.rows[44]["executable"] = {"bytes": 9, "sha256": "sh"}
        status = (4 << 16) | (backup.signal.SIGTRAP << 8) | 0x7f
        gate._event(44, status)
        self.assertEqual(gate.dump_pid, 44)
        self.assertEqual(gate.rows[44], process)
        with self.assertRaisesRegex(backup.PaperBackupError, "DUMP_GATE_CHILD"):
            gate._event(44, status)
        gate, _c, owner, process, _e, _b = self.gate()
        owner.ops._process_record.side_effect = lambda pid: dict(process, executable={"bytes": 7,"sha256":"forged"})
        with self.assertRaisesRegex(backup.PaperBackupError, "UNOWNED_CHILD"):
            gate._event(44, status)

    def test_periodic_observation_never_skips_a_held_syscall_boundary(self):
        gate, _c, owner, _p, _e, _b = self.gate()
        owner.revalidate.side_effect = None
        gate.syscall_stops.add(44)
        order = []
        with mock.patch.object(gate, "_stop_all", side_effect=lambda cleanup: order.append("hold")), \
                mock.patch.object(gate, "_syscall", side_effect=lambda pid: order.append("syscall")), \
                mock.patch.object(gate, "_resume", side_effect=lambda pid: order.append("resume")):
            gate.revalidate()
        self.assertEqual(order, ["hold", "syscall", "resume"])
        owner.revalidate.assert_called_once()

    def test_failed_detach_retains_tracees_and_original_budget(self):
        gate, client, owner, _p, _e, _b = self.gate()
        gate.stopped = {40, 44}
        gate.signals[44] = backup.signal.SIGTERM
        with mock.patch.object(gate, "_ptrace", side_effect=backup.PaperBackupError("PAPER_BACKUP_DUMP_GATE_KERNEL")), \
                self.assertRaises(backup.PaperBackupError):
            gate.close()
        self.assertFalse(gate.closed)
        self.assertEqual(set(gate.rows), {40,44})
        self.assertEqual(gate.signals[44], backup.signal.SIGTERM)
        self.assertIs(client.dump_gate, gate)
        with mock.patch.object(gate, "close", side_effect=backup.PaperBackupError("PAPER_BACKUP_DUMP_GATE_KERNEL")), \
                self.assertRaises(backup.PaperBackupError):
            client.close()
        self.assertIs(client.dump_gate, gate)
        self.assertFalse(client.absent)
        owner.context.docker.object.assert_not_called()

    def test_close_detaches_only_owned_generations_children_before_exporter(self):
        gate, _c, _o, _p, _e, _b = self.gate()
        gate.stopped = {40,44}
        gate.signals[44] = backup.signal.SIGTERM
        with mock.patch.object(gate, "_ptrace") as call:
            gate.close()
        self.assertEqual(call.call_args_list, [mock.call(gate.DETACH,44,backup.signal.SIGTERM),
                                               mock.call(gate.DETACH,40,0)])
        self.assertTrue(gate.closed)
        self.assertEqual(gate.rows, {})

    def test_deadline_failure_keeps_unknown_gate_and_never_extends_clock(self):
        gate, client, owner, _p, _e, _b = self.gate()
        deadline = owner._custody.deadline
        deadline.work_remaining.side_effect = backup.PaperBackupError("PAPER_OPS_ARCHIVE_TIMEOUT")
        with self.assertRaisesRegex(backup.PaperBackupError, "ARCHIVE_TIMEOUT"):
            gate.pump()
        deadline.remaining.side_effect = backup.PaperBackupError("PAPER_OPS_ARCHIVE_TIMEOUT")
        with self.assertRaisesRegex(backup.PaperBackupError, "ARCHIVE_TIMEOUT"):
            gate.close()
        self.assertIs(owner._custody.deadline, deadline)
        self.assertIs(client.dump_gate, gate)
        self.assertFalse(gate.closed)

    def test_arm_checks_original_exporter_before_seize_and_retains_partial_failure(self):
        gate, client, owner, _p, _e, _b = self.gate()
        client.dump_gate = None
        client.check = mock.Mock()
        owner.revalidate.side_effect = None
        events = []
        def ptrace(actual, request, pid, data=0, address=0):
            events.append((request,pid))
            if request == actual.INTERRUPT:
                raise backup.PaperBackupError("PAPER_BACKUP_DUMP_GATE_KERNEL")
            return 0
        with mock.patch.object(backup.ctypes,"CDLL"), \
                mock.patch.object(backup.os,"geteuid",return_value=0), \
                mock.patch.object(backup.platform,"machine",return_value="x86_64"), \
                mock.patch.object(backup.Path,"iterdir",return_value=[types.SimpleNamespace(name="40")]), \
                mock.patch.object(backup._DumpGate,"_ptrace",ptrace), \
                mock.patch.object(backup._DumpGate,"_poll",return_value=None), \
                self.assertRaisesRegex(backup.PaperBackupError,"DUMP_GATE_KERNEL"):
            backup._DumpGate(client)
        client.check.assert_called_once()
        owner.revalidate.assert_called_once()
        self.assertEqual(events, [(gate.SEIZE,40),(gate.INTERRUPT,40)])
        self.assertEqual(client.dump_gate.rows,{40:client.process})
        self.assertFalse(client.dump_gate.closed)

    def test_child_bound_failure_still_retains_new_kernel_tracee(self):
        gate, _c, owner, _p, _e, _b = self.gate()
        gate.rows = {pid:None for pid in range(100,164)}
        gate.rows[100] = gate.client.process
        def event_message(request,pid,data=0,address=0):
            backup.ctypes.cast(data, backup.ctypes.POINTER(backup.ctypes.c_ulong))[0] = 200
            return 0
        with mock.patch.object(gate,"_ptrace",side_effect=event_message), \
                self.assertRaisesRegex(backup.PaperBackupError,"DUMP_GATE_CHILD"):
            gate._fork(100)
        self.assertIn(200,gate.rows)
        self.assertIn(200,gate.pending)
        self.assertIsNone(gate.rows[200])
        owner.ops._process_record.assert_not_called()

    def test_failed_resume_preserves_pending_signal_and_stopped_identity(self):
        gate, _c, _o, _p, _e, _b = self.gate()
        gate.signals[44] = backup.signal.SIGTERM
        gate.syscall_stops.add(44)
        with mock.patch.object(gate,"_ptrace",side_effect=backup.PaperBackupError("PAPER_BACKUP_DUMP_GATE_KERNEL")), \
                self.assertRaises(backup.PaperBackupError):
            gate._resume(44)
        self.assertEqual(gate.signals[44],backup.signal.SIGTERM)
        self.assertIn(44,gate.stopped)
        self.assertIn(44,gate.syscall_stops)

    def test_fork_event_retains_kernel_child_before_validation_failure(self):
        for foreign in (False, True):
            gate, _client, owner, process, _e, _b = self.gate()
            gate.rows.pop(44)
            gate.stopped = {40}
            child = dict(process, executable=gate.rows[40]["executable"],
                         cgroup="/foreign" if foreign else "/owned")
            owner.ops._process_record.side_effect = lambda pid: child
            def event_message(request, pid, data=0, address=0):
                self.assertEqual((request,pid), (gate.GETEVENTMSG,40))
                backup.ctypes.cast(data, backup.ctypes.POINTER(backup.ctypes.c_ulong))[0] = 44
                return 0
            with mock.patch.object(gate, "_ptrace", side_effect=event_message):
                if foreign:
                    with self.assertRaisesRegex(backup.PaperBackupError, "UNOWNED_CHILD"):
                        gate._fork(40)
                else:
                    gate._fork(40)
            self.assertEqual(gate.rows[44], child)
            self.assertIn(44, gate.pending)

    def test_preexec_exporter_elf_requires_exact_kernel_fork_custody(self):
        gate, client, owner, process, _e, _b = self.gate()
        child = dict(process, executable=client.process["executable"])
        gate.rows[44] = child
        owner.ops._process_record.side_effect = lambda pid: child
        owner.ops._proc_cgroups = lambda: {40: "/owned", 44: "/owned"}
        self.assertEqual(backup._Client.child_processes(client), {44:child})
        for changed in (dict(child,start_ticks=56), dict(child,parent=39)):
            owner.ops._process_record.side_effect = lambda pid: changed
            with self.assertRaises(backup.PaperBackupError):
                backup._Client.child_processes(client)
        owner.ops._process_record.side_effect = lambda pid: child
        gate.rows.pop(44)
        with self.assertRaisesRegex(backup.PaperBackupError, "UNOWNED_CHILD"):
            backup._Client.child_processes(client)

    def test_non_database_socket_is_not_capture_or_a_close_waiver(self):
        gate, client, owner, _p, endpoint, _b = self.gate()
        owner.ops._tcp_endpoints.return_value = [dict(endpoint,peer=["10.0.0.1",53])]
        with mock.patch.object(backup.os, "readlink", return_value="socket:[123]"):
            gate._socket_boundary(44,6,terminal=True)
        owner.revalidate.assert_not_called()
        self.assertFalse(client.dump_backends)
        with self.syscall(gate,1,231), self.assertRaisesRegex(backup.PaperBackupError,"DUMP_BACKEND_UNOBSERVED"):
            gate._syscall(44)

    def test_client_disposal_requires_independent_absence_of_dump_backend(self):
        gate, client, owner, process, endpoint, backend = self.gate()
        client.dump_gate = None
        client.sent, client.cid, client.created = True, "owned", "created"
        client.generation = ("owned-generation",)
        client.dump_backends = {11: (backend,process,endpoint)}
        owner.ops._container_generation = lambda value: client.generation
        owner.ops._terminal_container = lambda value,generation: "terminal"
        owner.ops._proc_cgroups = lambda: {}
        owner.context.docker.object.return_value = {"State":{"Running":False}}
        owner._cleanup_docker = mock.Mock(return_value="")
        owner._custody.observer = types.SimpleNamespace(database=mock.Mock(return_value={"backends":[backend]}))
        with mock.patch.object(client,"policy"), self.assertRaisesRegex(backup.PaperBackupError,"CLEANUP_UNKNOWN"):
            client.close()
        owner._cleanup_docker.assert_not_called()
        self.assertFalse(client.absent)
        owner._custody.observer.database.return_value = {"backends":[]}
        with mock.patch.object(client,"policy"):
            client.close()
        self.assertTrue(client.absent)
        self.assertEqual(owner._cleanup_docker.call_args_list[0], mock.call(["rm","owned"]))

    def test_unobserved_tracee_exit_never_satisfies_backend_capture(self):
        gate, client, _o, _p, _e, _b = self.gate()
        with self.assertRaisesRegex(backup.PaperBackupError, "DUMP_BACKEND_UNOBSERVED"):
            gate._event(44, 0)
        self.assertEqual(client.dump_backends, {})
        self.assertFalse(gate.closed)


class ExchangeCleanupTests(unittest.TestCase):
    def client(self):
        owner = types.SimpleNamespace(maximum=10000,
            _custody=types.SimpleNamespace(deadline=types.SimpleNamespace(work_remaining=lambda cap=15: cap)))
        client = backup._Client(owner, "exporter")
        client.dumping, client.backend = True, {"pid": 10}
        client.child = types.SimpleNamespace(poll=lambda: None,
            stdin=types.SimpleNamespace(fileno=lambda: 3), stdout=types.SimpleNamespace(fileno=lambda: 4))
        return client

    def test_primary_failure_survives_failed_gate_cleanup_and_fd_restore(self):
        for primary in (backup.PaperBackupError("PRIMARY"), KeyboardInterrupt()):
            with self.subTest(primary=type(primary).__name__):
                client = self.client()
                gate = types.SimpleNamespace(closed=False, pump=mock.Mock(side_effect=primary),
                    close=mock.Mock(side_effect=backup.PaperBackupError("CLEANUP")))
                def arm(actual):
                    actual.dump_gate = gate
                    return gate
                def blocking(fd, enabled):
                    if enabled:
                        raise OSError("RESTORE")
                with mock.patch.object(backup, "_DumpGate", side_effect=arm), \
                        mock.patch.object(backup.os, "set_blocking", side_effect=blocking) as restore, \
                        self.assertRaises(type(primary)) as caught:
                    client.exchange(b"actual-source-command")
                self.assertIs(caught.exception, primary)
                self.assertIs(client.dump_gate, gate)
                gate.close.assert_called_once_with()
                self.assertEqual(restore.call_args_list[-2:], [mock.call(3, True), mock.call(4, True)])

    def test_completed_exchange_does_not_suppress_gate_cleanup_failure(self):
        client = self.client()
        gate = types.SimpleNamespace(closed=True, pump=mock.Mock(),
            close=mock.Mock(side_effect=backup.PaperBackupError("CLEANUP")))
        written = []
        def arm(actual):
            actual.dump_gate = gate
            actual.dump_backends[11] = ("component-only",)
            return gate
        def write(fd, data):
            written.append(bytes(data))
            return len(data)
        def read(fd, size):
            marker = written[0].split(b"\\echo ")[1].splitlines()[0]
            return b"PGDMPshort\n" + marker + b"\n"
        with mock.patch.object(backup, "_DumpGate", side_effect=arm), \
                mock.patch.object(backup.os, "set_blocking"), \
                mock.patch.object(backup.os, "write", side_effect=write), \
                mock.patch.object(backup.os, "read", side_effect=read), \
                mock.patch.object(backup.select, "select", return_value=([4], [3], [])), \
                self.assertRaisesRegex(backup.PaperBackupError, "CLEANUP"):
            client.exchange(b"actual-source-command")
        self.assertIs(client.dump_gate, gate)

    def test_first_fd_restore_error_is_preserved_and_second_restore_is_attempted(self):
        client = self.client()
        first, second = OSError("FIRST_RESTORE"), OSError("SECOND_RESTORE")
        gate = types.SimpleNamespace(closed=True, pump=mock.Mock(), close=mock.Mock())
        written = []
        def arm(actual):
            actual.dump_gate = gate
            actual.dump_backends[11] = ("component-only",)
            return gate
        def write(fd, data):
            written.append(bytes(data))
            return len(data)
        def read(fd, size):
            marker = written[0].split(b"\\echo ")[1].splitlines()[0]
            return marker + b"\n"
        def blocking(fd, enabled):
            if enabled:
                raise first if fd == 3 else second
        with mock.patch.object(backup, "_DumpGate", side_effect=arm), \
                mock.patch.object(backup.os, "set_blocking", side_effect=blocking) as restore, \
                mock.patch.object(backup.os, "write", side_effect=write), \
                mock.patch.object(backup.os, "read", side_effect=read), \
                mock.patch.object(backup.select, "select", return_value=([4], [3], [])), \
                self.assertRaises(OSError) as caught:
            client.exchange(b"actual-source-command")
        self.assertIs(caught.exception, first)
        self.assertEqual(restore.call_args_list[-2:], [mock.call(3, True), mock.call(4, True)])
        gate.close.assert_called_once_with()
        self.assertIsNone(client.dump_gate)


if __name__ == "__main__":
    unittest.main()
