#!/usr/bin/env python3
"""Focused stdlib tests for the pre-switch Paper history inventory (Option A).

Filesystem predicates, descriptors, locks and receipts are real (temporary
trees). Only root ownership/ACL admission, the Docker/DB transport, the
protected env parser and the P4 INSPECT probe are seamed.
"""
from __future__ import annotations

import contextlib
import copy
import datetime
import fcntl
import hashlib
import importlib.util
import io
import json
import os
import pickle
import re
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
# Reuse an already-loaded release test module: executing it again would
# re-register its release copy and break that module's own pickle identity test.
base = (sys.modules.get("scripts.ops.test_owner_beta_paper_release")
        or sys.modules.get("test_owner_beta_paper_release"))
if base is None:
    sys.path.insert(0, str(HERE))
    import test_owner_beta_paper_release as base  # noqa: E402

release = base.release
ROOT = base.ROOT
COMMIT = base.COMMIT
SOURCE = "b2" * 20
inventory = release._history_inventory()
ops = inventory._ops()
daily = release._daily()
CHECKSUMS = inventory.migration_checksums()
EXPECTED = inventory.expected_lines(CHECKSUMS)
PROJECT = "lagrange-station"
DB_IMAGE = "sha256:" + "e" * 64
NETWORK = "f" * 64
GENERATION = ("c" * 64, "2026-11-17T00:00:00.000000000Z", "2026-11-17T00:00:01.000000000Z", 0, DB_IMAGE)
RECEIPT_KEYS = {
    "schema", "outcome", "inventory", "state", "install_root", "install_root_identity",
    "release_lock_identity", "source_commit", "target_commit", "source_manifest_sha256",
    "target_manifest_sha256", "inventory_sql_sha256", "migration_max",
    "migration_lineage_sha256", "database", "p4_index_sha256", "local", "boot_id", "observed_at",
}


def line_for(state, version, checksums=CHECKSUMS):
    parts = [str(v) + ":" + d for v, d in checksums if v <= version]
    lineage = hashlib.sha256(",".join(parts).encode("ascii")).hexdigest()
    return "PAPER_HISTORY_INVENTORY: %s max=%d lineage=%s" % (state, version, lineage)


PRE_LINE = line_for("PRE_PAPER", 65)
SCHEMA_LINE = line_for("PAPER_SCHEMA", 69)


def mkdir(path, mode):
    path.mkdir()
    os.chmod(path, mode)


class FakeProbe:
    def __init__(self, world):
        self.world = world
        self._index = b'{"control_epoch":0,"fixture":"p4-index"}'
        self.closed = False
        self.revalidations = 0

    def revalidate(self):
        if self.world.probe_error is not None:
            raise self.world.probe_error
        self.revalidations += 1

    def _consume_presence(self, token):
        if token is not ops._READER_CONSUMER:
            raise AssertionError("wrong consumer token")
        self.revalidate()
        return self.world.presence

    def close(self):
        # The real P4 local markers refuse to close once a floor appeared.
        if (self.world.install / release.FLOOR_NAME).exists():
            raise ops.PaperOperationsError("PAPER_OPS_LOCAL_CHANGED")
        self.closed = True


class FakeContext:
    def __init__(self, world, context, lock_fd):
        self.world, self.context, self.lock_fd = world, context, lock_fd
        self.probe = None
        self.closed = False

    def probe_history(self):
        if self.world.probe_error is not None:
            raise self.world.probe_error
        self.probe = FakeProbe(self.world)
        return self.probe

    def close(self):
        if self.probe is not None and not self.probe.closed:
            raise ops.PaperOperationsError("PAPER_OPS_INVENTORY_OPEN")
        self.closed = True


class World:
    """A production-shaped install root plus seams for one test."""

    def __init__(self, base_path, *, state="PRE_PAPER", source_capable=None, lifecycle=False):
        self.base = base_path
        self.tree = base.build_release_tree(base_path)
        self.install, self.target = self.tree.install, self.tree.target
        self.source = self.install / "releases" / SOURCE
        mkdir(self.source, 0o755)
        mkdir(self.source / "deploy", 0o755)
        (self.source / ".lagrange-release-manifest").write_bytes(b"source manifest fixture\n")
        os.chmod(self.source / ".lagrange-release-manifest", 0o600)
        if source_capable is None:
            source_capable = state == "PAPER_SCHEMA"
        if source_capable:
            self.add_source_paper("capability.json")
        os.symlink("releases/" + SOURCE, self.install / "current")
        lock = self.install / ".lagrange-release.lock"
        lock.write_bytes(b"")
        os.chmod(lock, 0o600)
        self.lock_fd = os.open(lock, os.O_RDWR | os.O_CLOEXEC)
        fcntl.flock(self.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        self.data = base_path / "data"
        mkdir(self.data, 0o755)
        if lifecycle:
            self.add_lifecycle()
        self.state = state
        self.line = SCHEMA_LINE if state == "PAPER_SCHEMA" else PRE_LINE
        self.sql_error = None
        self.generations = None
        self.networks = None
        self.presence = ops._HISTORY_EMPTY
        self.probe_error = None
        self.modes = {str(self.source): "disabled", str(self.target): ""}
        self.execute_calls = []
        self.contexts = []

    def close(self):
        os.close(self.lock_fd)

    def add_source_paper(self, *names):
        paper = self.source / "deploy" / "owner-beta-paper"
        if not paper.exists():
            mkdir(paper, 0o750)
        for name in names:
            (paper / name).write_bytes(b"{}")
            os.chmod(paper / name, 0o444)

    def add_lifecycle(self, mode=0o700):
        root = self.install / ".owner-beta-paper-lifecycle"
        mkdir(root, mode)
        for name in ("queries", "selections", "anchors", "staging"):
            mkdir(root / name, 0o700)
        return root

    def values(self, release_path):
        return {
            "LAGRANGE_CODE_COMMIT": SOURCE if release_path == str(self.source) else COMMIT,
            "LAGRANGE_DATA_DIR": str(self.data), "LAGRANGE_ARTIFACTS_DIR": "/data/artifacts",
            "LAGRANGE_RUNTIME_SECRET_DIR": "/run/secrets", "POSTGRES_DB": "lagrange",
            "MAX_JOBS_PER_OWNER": "10", "PRICE_BETA_LEASE_SECS": "60",
            "PRICE_BETA_BACKOFF_SECS": "30", "OWNER_BETA_DAILY_MODE": "disabled",
            "OWNER_BETA_ACCESS_MODE": "disabled", "OWNER_BETA_PRICE_INPUT_MODE": "disabled",
            "OWNER_BETA_PAPER_MODE": self.modes.get(release_path, "disabled"),
        }

    def read_context(self, release_path, commit, action, *, expected_env_sha256=None):
        if action != "disable":
            raise AssertionError("inventory must read the disable context")
        if release_path not in (str(self.source), str(self.target)) and \
                not release_path.startswith(str(self.install / "releases" / (".staging." + COMMIT + "."))):
            raise AssertionError("unexpected release " + release_path)
        return self.values(release_path), []

    def read_manifest(self, release_path, commit):
        if (release_path, commit) != (str(self.source), SOURCE):
            raise AssertionError("inventory must bind the source (current) manifest")
        return {"db-migrate": "sha256:" + "d" * 64, "paper-scheduler": "sha256:" + "a" * 64}

    def project_binding(self, source_commit, target_commit):
        if (source_commit, target_commit) != (SOURCE, COMMIT):
            raise AssertionError("project binding must name source and target")
        return PROJECT, "absent"

    def capture(self, docker, project, image):
        if project != PROJECT or image != DB_IMAGE:
            raise AssertionError("capture binding")
        generation = self.generations.pop(0) if self.generations else GENERATION
        network = self.networks.pop(0) if self.networks else NETWORK
        return network, generation

    def execute(self, database, values, sql, expected, *, backup_audit=False):
        self.execute_calls.append((database.project, database.commit, database.image, sql))
        if self.sql_error is not None:
            raise self.sql_error
        if self.line not in expected:
            raise ops.daily.runtime.RuntimeErrorCode("DAILY_OPERATION_DATABASE_RESULT")
        return self.line

    def open_context(self, context, lock_fd):
        if type(context) is not ops.daily.Context or context.commit != SOURCE:
            raise AssertionError("P4 must be hosted by the current release")
        value = FakeContext(self, context, lock_fd)
        self.contexts.append(value)
        return value

    def bound_execute(self):
        world = self

        def execute(database, values, sql, expected, *, backup_audit=False):
            return world.execute(database, values, sql, expected, backup_audit=backup_audit)
        return execute

    @contextlib.contextmanager
    def seams(self):
        with base.portable_release_seams(daily, self.tree), \
                mock.patch.object(inventory, "_docker", return_value=object()), \
                mock.patch.object(ops.daily, "read_manifest", side_effect=self.read_manifest), \
                mock.patch.object(ops.daily, "read_context", side_effect=self.read_context), \
                mock.patch.object(ops.daily, "project_binding", side_effect=self.project_binding), \
                mock.patch.object(ops.daily.Database, "execute", new=self.bound_execute()), \
                mock.patch.object(ops, "_database_image_id", return_value=DB_IMAGE), \
                mock.patch.object(ops, "_capture_database", side_effect=self.capture), \
                mock.patch.object(ops, "open_context", side_effect=self.open_context):
            yield

    def prove(self, **kwargs):
        return inventory.prove(str(self.install), str(self.target), COMMIT, self.lock_fd, **kwargs)

    def guard(self, inv, reserve=True):
        return release.reader_guard(str(self.install), str(self.target), COMMIT, None, reserve,
                                    inventory=inv)

    def receipts(self):
        directory = self.install / inventory.RECEIPT_DIRECTORY
        return sorted(directory.iterdir()) if directory.exists() else []

    def floor(self):
        return (self.install / release.FLOOR_NAME).exists()


class InventoryCase(unittest.TestCase):
    def world(self, **kwargs):
        temporary = tempfile.TemporaryDirectory(prefix="paper-history-inventory-")
        self.addCleanup(temporary.cleanup)
        world = World(Path(temporary.name), **kwargs)
        self.addCleanup(world.close)
        return world

    def assert_inventory_error(self, code, function, *args, **kwargs):
        with self.assertRaises(inventory.PaperHistoryInventoryError) as caught:
            function(*args, **kwargs)
        self.assertEqual(caught.exception.code, code)

    def assert_unknown(self, function, *args, **kwargs):
        with self.assertRaises(release.PaperReleaseError) as caught:
            function(*args, **kwargs)
        self.assertEqual(caught.exception.code, "PAPER_RELEASE_HISTORY_UNKNOWN")
        return caught.exception

    def assert_cli_unknown(self, world, *, prove_code=None):
        """The deploy CLI path: prove and guard fail UNKNOWN; no floor, no receipt."""
        error = self.assert_unknown(release._reader_guard_with_inventory, str(world.install),
                                    str(world.target), COMMIT, world.lock_fd, True)
        if prove_code is not None:
            self.assertEqual(error.diagnostic, {"stage": "READER_GUARD", "inventory": prove_code})
        self.assertFalse(world.floor())
        self.assertEqual(world.receipts(), [])


class StateATests(InventoryCase):
    def check_receipt(self, world, path, *, state="PRE_PAPER", lifecycle="ABSENT"):
        info = path.stat()
        self.assertEqual(stat.S_IMODE(info.st_mode), 0o400)
        self.assertEqual(stat.S_IMODE((world.install / inventory.RECEIPT_DIRECTORY).stat().st_mode), 0o700)
        self.assertTrue(re.fullmatch(r"[0-9]{8}T[0-9]{6}Z-" + COMMIT + r"\.json", path.name))
        raw = path.read_bytes()
        self.assertTrue(raw.endswith(b"\n") and len(raw) <= 4096)
        value = json.loads(raw)
        self.assertEqual(raw, release._canonical(value) + b"\n")
        self.assertEqual(set(value), RECEIPT_KEYS)
        self.assertEqual(value["schema"], "owner-beta-paper-history-inventory-receipt-v1")
        self.assertEqual((value["outcome"], value["inventory"], value["state"]),
                         ("LEGACY_OFF", "ABSENT", state))
        self.assertEqual((value["source_commit"], value["target_commit"]), (SOURCE, COMMIT))
        self.assertEqual(value["install_root"], str(world.install))
        root = world.install.stat()
        self.assertEqual(value["install_root_identity"], [root.st_dev, root.st_ino])
        lock = os.fstat(world.lock_fd)
        self.assertEqual(value["release_lock_identity"], [lock.st_dev, lock.st_ino])
        sql = (ROOT / inventory.SQL_RELATIVE).read_bytes()
        self.assertEqual(value["inventory_sql_sha256"], "sha256:" + hashlib.sha256(sql).hexdigest())
        self.assertEqual(value["target_manifest_sha256"], "sha256:" + hashlib.sha256(
            (world.target / ".lagrange-release-manifest").read_bytes()).hexdigest())
        self.assertEqual(value["source_manifest_sha256"], "sha256:" + hashlib.sha256(
            b"source manifest fixture\n").hexdigest())
        expected_line = world.line.split(" ")
        self.assertEqual(value["migration_max"], int(expected_line[2][4:]))
        self.assertEqual(value["migration_lineage_sha256"], expected_line[3][8:])
        self.assertEqual(value["database"], {
            "container_id": GENERATION[0], "created": GENERATION[1], "started_at": GENERATION[2],
            "restart_count": 0, "image_id": DB_IMAGE, "network_id": NETWORK, "database": "lagrange"})
        self.assertEqual(value["local"], {
            "paper_dir": False, "reader_floor": False, "lifecycle": lifecycle,
            "source_binding": False, "target_binding": False,
            "evidence_root": "ABSENT" if state == "PRE_PAPER" else "NOT_CHECKED",
            "source_paper_mode": "disabled", "target_paper_mode": ""})
        self.assertEqual(value["boot_id"], inventory._boot_id())
        self.assertTrue(re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{6}Z", value["observed_at"]))
        for secret in ("/run/secrets", "LAGRANGE_DATA_DIR", str(world.data)):
            self.assertNotIn(secret, raw.decode("utf-8"))
        return value

    def test_pre_paper_absent_reaches_legacy_off_with_one_receipt_and_no_floor(self):
        for lifecycle in (False, True):
            with self.subTest(lifecycle=lifecycle):
                world = self.world(lifecycle=lifecycle)
                with world.seams():
                    inv = world.prove()
                    try:
                        self.assertEqual((inv.outcome, inv.state), ("ABSENT", "PRE_PAPER"))
                        self.assertEqual(world.guard(inv), "LEGACY_OFF")
                    finally:
                        inv.close()
                self.assertFalse(world.floor())
                receipts = world.receipts()
                self.assertEqual(len(receipts), 1)
                self.check_receipt(world, receipts[0],
                                   lifecycle="EMPTY_SKELETON" if lifecycle else "ABSENT")
                # prove, consume revalidation and the final revalidation each
                # rerun the same fixed SQL through the source db-migrate image.
                sql = (ROOT / inventory.SQL_RELATIVE).read_text("ascii")
                self.assertEqual(world.execute_calls,
                                 [(PROJECT, SOURCE, "sha256:" + "d" * 64, sql)] * 3)
                # The receipt directory never counts as a Paper marker.
                self.assertFalse(inventory.RECEIPT_DIRECTORY.startswith(inventory.PAPER_PREFIX))

    def test_second_guard_writes_a_second_receipt_and_never_replaces(self):
        world = self.world()
        first = datetime.datetime(2026, 11, 17, 1, 2, 3, 456789, tzinfo=datetime.timezone.utc)
        second = first + datetime.timedelta(seconds=1)
        with world.seams():
            for now in (first, second):
                with mock.patch.object(inventory, "_utc_now", return_value=now):
                    inv = world.prove()
                    try:
                        self.assertEqual(world.guard(inv), "LEGACY_OFF")
                    finally:
                        inv.close()
            names = [path.name for path in world.receipts()]
            self.assertEqual(names, ["20261117T010203Z-" + COMMIT + ".json",
                                     "20261117T010204Z-" + COMMIT + ".json"])
            original = world.receipts()[1].read_bytes()
            with mock.patch.object(inventory, "_utc_now", return_value=second):
                inv = world.prove()
                try:
                    error = self.assert_unknown(world.guard, inv)
                finally:
                    inv.close()
            self.assertEqual(error.diagnostic, {"stage": "READER_GUARD", "inventory": "PAPER_INVENTORY_RECEIPT"})
        self.assertEqual(len(world.receipts()), 2)
        self.assertEqual(world.receipts()[1].read_bytes(), original)
        self.assertFalse(world.floor())

    def test_simulation_and_daily_floor_guards_still_run_before_receipt(self):
        world = self.world()
        for name in (".owner-beta-simulation-reader-floor", ".owner-beta-daily-reader-floor"):
            (world.install / name).write_bytes(b"{}")
        sim, daily_ops = mock.Mock(), mock.Mock()
        with world.seams(), mock.patch.object(release, "_simulation_release", return_value=sim), \
                mock.patch.object(release, "_daily_operations", return_value=daily_ops):
            inv = world.prove()
            try:
                self.assertEqual(world.guard(inv), "LEGACY_OFF")
            finally:
                inv.close()
            sim.reader_guard.assert_called_once_with(str(world.install), str(world.target), COMMIT,
                                                     "disabled", history_present=True, reserve=False)
            daily_ops.install.reader_guard.assert_called_once_with(
                str(world.install), str(world.target), "disabled", reserve=False)
            self.assertEqual(len(world.receipts()), 1)
            sim.reader_guard.side_effect = release.PaperReleaseError("SIMULATION_READER_REQUIRED")
            inv = world.prove()
            try:
                with self.assertRaises(release.PaperReleaseError) as caught:
                    world.guard(inv)
            finally:
                inv.close()
            self.assertEqual(caught.exception.code, "SIMULATION_READER_REQUIRED")
        self.assertEqual(len(world.receipts()), 1)
        self.assertFalse(world.floor())

    def test_check_mode_without_reservation_writes_no_receipt(self):
        world = self.world()
        with world.seams():
            inv = world.prove()
            try:
                self.assertEqual(world.guard(inv, reserve=False), "LEGACY_OFF")
            finally:
                inv.close()
        self.assertEqual(world.receipts(), [])
        self.assertFalse((world.install / inventory.RECEIPT_DIRECTORY).exists())
        self.assertFalse(world.floor())

    def test_cli_path_proves_and_consumes_in_one_process(self):
        world = self.world()
        with world.seams():
            self.assertEqual(release._reader_guard_with_inventory(
                str(world.install), str(world.target), COMMIT, world.lock_fd, True), "LEGACY_OFF")
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                result = release.main(["reader-guard", "--install-root", str(world.install),
                                       "--target-release", str(world.target), "--commit", COMMIT,
                                       "--no-reserve", "--history-inventory",
                                       "--lock-fd", str(world.lock_fd)])
            self.assertEqual((result, stdout.getvalue()), (0, "PAPER_READER_GUARD: LEGACY_OFF\n"))
        self.assertEqual(len(world.receipts()), 1)
        self.assertFalse(world.floor())
        # The caller's lock descriptor stays open and locked.
        fcntl.flock(world.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)


class LocalPredicateTests(InventoryCase):
    def mutate_cases(self):
        def lifecycle_file(world):
            root = world.add_lifecycle()
            (root / "queries" / "q.json").write_bytes(b"{}")

        def lifecycle_extra(world):
            mkdir(world.add_lifecycle() / "extra", 0o700)

        def lifecycle_mode(world):
            world.add_lifecycle(mode=0o755)

        def lifecycle_missing_member(world):
            os.rmdir(world.add_lifecycle() / "staging")

        def unknown_name(world):
            mkdir(world.install / ".owner-beta-paper-x", 0o700)

        def evidence_root(world):
            mkdir(world.data / "owner-beta-paper-evidence", 0o700)

        def source_process(world):
            world.add_source_paper("capability.json", "process.json")

        def source_config(world):
            world.add_source_paper("config.json")

        def source_mode(world):
            world.modes[str(world.source)] = "owner_only"

        def target_mode(world):
            world.modes[str(world.target)] = "owner_only"

        def no_current(world):
            os.unlink(world.install / "current")

        def foreign_current(world):
            os.unlink(world.install / "current")
            os.symlink("../elsewhere", world.install / "current")

        def lifecycle_symlink(world):
            os.symlink("releases", world.install / ".owner-beta-paper-lifecycle")

        return {"lifecycle_file": lifecycle_file, "lifecycle_extra": lifecycle_extra,
                "lifecycle_mode": lifecycle_mode, "lifecycle_missing_member": lifecycle_missing_member,
                "unknown_name": unknown_name, "evidence_root": evidence_root,
                "source_process": source_process, "source_config": source_config,
                "source_mode": source_mode, "target_mode": target_mode, "no_current": no_current,
                "foreign_current": foreign_current, "lifecycle_symlink": lifecycle_symlink}

    def test_each_local_marker_is_unknown_without_floor_or_receipt(self):
        for name, mutate in self.mutate_cases().items():
            with self.subTest(case=name):
                world = self.world()
                mutate(world)
                with world.seams():
                    self.assert_inventory_error("PAPER_INVENTORY_LOCAL", world.prove)
                    self.assert_cli_unknown(world, prove_code="PAPER_INVENTORY_LOCAL")

    def test_local_change_after_proof_fails_at_consume(self):
        for name, mutate in self.mutate_cases().items():
            if name in ("no_current", "foreign_current"):
                continue
            with self.subTest(case=name):
                world = self.world()
                with world.seams():
                    inv = world.prove()
                    try:
                        mutate(world)
                        self.assert_unknown(world.guard, inv)
                    finally:
                        inv.close()
                self.assertFalse(world.floor())
                self.assertEqual(world.receipts(), [])

    def test_existing_markers_require_reader_without_consulting_inventory(self):
        cases = ("floor", "paper_dir", "target_binding")
        for case in cases:
            with self.subTest(case=case):
                world = self.world()
                with world.seams():
                    inv = world.prove()
                    try:
                        if case == "floor":
                            floor = world.install / release.FLOOR_NAME
                            floor.write_bytes(release.FLOOR_BYTES)
                            os.chmod(floor, 0o600)
                        elif case == "paper_dir":
                            mkdir(world.install / ".owner-beta-paper", 0o700)
                        else:
                            (world.target / "deploy/owner-beta-paper/process.json").write_bytes(b"{}")
                        handle = mock.Mock()
                        with mock.patch.object(release, "verify", return_value=handle) as verify:
                            self.assertEqual(world.guard(inv), "PAPER_READER_REQUIRED")
                        verify.assert_called_once_with(str(world.install), str(world.target), COMMIT)
                        handle.close.assert_called_once_with()
                        self.assertFalse(object.__getattribute__(inv, "_consumed"))
                    finally:
                        inv.close()
                self.assertTrue(world.floor())
                self.assertEqual(world.receipts(), [])


class SqlTests(InventoryCase):
    def test_sql_failures_and_foreign_lines_are_unknown(self):
        foreign = [(v, "0" * 96 if v == 3 else d) for v, d in CHECKSUMS]
        newer = list(CHECKSUMS) + [(70, "1" * 96)]
        cases = {
            "execute_raises": ("PAPER_INVENTORY_SQL",
                               {"sql_error": ops.daily.runtime.RuntimeErrorCode("DAILY_OPERATION_DATABASE")}),
            "timeout": ("PAPER_INVENTORY_SQL",
                        {"sql_error": subprocess.TimeoutExpired(["docker"], 45)}),
            "partial_line": ("PAPER_INVENTORY_SQL",
                             {"line": "PAPER_HISTORY_INVENTORY: PARTIAL max=67 lineage=" + "0" * 64}),
            "foreign_lineage": ("PAPER_INVENTORY_SQL", {"line": line_for("PRE_PAPER", 65, foreign)}),
            "newer_than_code": ("PAPER_INVENTORY_SQL", {"line": line_for("PAPER_SCHEMA", 70, newer)}),
            "state_a_above_65": ("PAPER_INVENTORY_SQL", {"line": line_for("PRE_PAPER", 66)}),
            "generation_differs": ("PAPER_INVENTORY_GENERATION",
                                   {"generations": [GENERATION, GENERATION[:3] + (1, DB_IMAGE)]}),
            "network_differs": ("PAPER_INVENTORY_GENERATION", {"networks": [NETWORK, "9" * 64]}),
        }
        for name, (code, changes) in cases.items():
            with self.subTest(case=name):
                world = self.world()
                with world.seams():
                    for key, value in changes.items():
                        setattr(world, key, list(value) if type(value) is list else value)
                    self.assert_inventory_error(code, world.prove)
                    for key, value in changes.items():
                        setattr(world, key, list(value) if type(value) is list else value)
                    self.assert_cli_unknown(world, prove_code=code)

    def test_database_change_between_proof_and_consume_is_unknown(self):
        for name in ("line", "generation", "sql_error"):
            with self.subTest(case=name):
                world = self.world()
                with world.seams():
                    inv = world.prove()
                    try:
                        if name == "line":
                            world.line = line_for("PRE_PAPER", 64)
                        elif name == "generation":
                            world.generations = [GENERATION[:3] + (1, DB_IMAGE)] * 2
                        else:
                            world.sql_error = ops.daily.runtime.RuntimeErrorCode("DAILY_OPERATION_DATABASE")
                        self.assert_unknown(world.guard, inv)
                    finally:
                        inv.close()
                self.assertFalse(world.floor())
                self.assertEqual(world.receipts(), [])

    def test_no_provider_or_sql_text_crosses_the_cli(self):
        world = self.world()
        world.sql_error = RuntimeError("SECRET_SENTINEL psql: password authentication failed")
        stderr, stdout = io.StringIO(), io.StringIO()
        with world.seams(), contextlib.redirect_stderr(stderr), contextlib.redirect_stdout(stdout):
            result = release.main(["reader-guard", "--install-root", str(world.install),
                                   "--target-release", str(world.target), "--commit", COMMIT,
                                   "--history-inventory", "--lock-fd", str(world.lock_fd)])
        self.assertEqual(result, 1)
        self.assertEqual(stdout.getvalue(), "")
        lines = stderr.getvalue().splitlines()
        self.assertEqual(lines[0], "PAPER_RELEASE_HISTORY_UNKNOWN")
        self.assertEqual(json.loads(lines[1].split(":", 1)[1]),
                         {"stage": "READER_GUARD", "inventory": "PAPER_INVENTORY_SQL"})
        self.assertEqual(len(lines), 2)
        self.assertNotIn("SECRET_SENTINEL", stderr.getvalue())


class StateBTests(InventoryCase):
    def test_empty_probe_is_absent_and_legacy_off(self):
        world = self.world(state="PAPER_SCHEMA")
        with world.seams():
            inv = world.prove()
            try:
                self.assertEqual((inv.outcome, inv.state, inv.migration_max), ("ABSENT", "PAPER_SCHEMA", 69))
                self.assertEqual(world.guard(inv), "LEGACY_OFF")
            finally:
                inv.close()
        self.assertFalse(world.floor())
        context = world.contexts[0]
        self.assertEqual((context.context.release, context.context.commit), (str(world.source), SOURCE))
        self.assertTrue(context.closed and context.probe.closed)
        self.assertEqual(len(world.contexts), 1)
        receipts = world.receipts()
        self.assertEqual(len(receipts), 1)
        value = StateATests.check_receipt(self, world, receipts[0], state="PAPER_SCHEMA")
        self.assertEqual(value["p4_index_sha256"], "sha256:" + hashlib.sha256(
            context.probe._index).hexdigest())

    def test_present_probe_requires_reader_reserves_floor_and_verifies(self):
        world = self.world(state="PAPER_SCHEMA")
        world.presence = ops._HISTORY_PRESENT
        with world.seams(), mock.patch.object(release, "verify", wraps=release.verify) as verify:
            inv = world.prove()
            try:
                self.assertEqual(inv.outcome, "PRESENT")
                self.assertEqual(world.guard(inv), "PAPER_READER_REQUIRED")
            finally:
                inv.close()
        verify.assert_called_once_with(str(world.install), str(world.target), COMMIT)
        self.assertEqual((world.install / release.FLOOR_NAME).read_bytes(), release.FLOOR_BYTES)
        self.assertEqual(world.receipts(), [])
        # The P4 probe was released before the floor appeared.
        self.assertTrue(world.contexts[0].closed and world.contexts[0].probe.closed)

    def test_probe_failure_or_missing_source_capability_is_unknown(self):
        cases = {
            "probe_raises": ("PAPER_INVENTORY_PROBE", {"probe_error": ops.PaperOperationsError("PAPER_OPS_HISTORY_UNKNOWN")}),
            "unknown_presence": ("PAPER_INVENTORY_PROBE", {"presence": object()}),
            "source_not_capable": ("PAPER_INVENTORY_SOURCE", {}),
        }
        for name, (code, changes) in cases.items():
            with self.subTest(case=name):
                world = self.world(state="PAPER_SCHEMA", source_capable=name != "source_not_capable")
                for key, value in changes.items():
                    setattr(world, key, value)
                with world.seams():
                    self.assert_inventory_error(code, world.prove)
                    self.assert_cli_unknown(world, prove_code=code)
                self.assertTrue(all(context.closed or context.probe is None for context in world.contexts))

    def test_probe_drift_after_proof_is_unknown(self):
        world = self.world(state="PAPER_SCHEMA")
        with world.seams():
            inv = world.prove()
            try:
                world.probe_error = ops.PaperOperationsError("PAPER_OPS_HISTORY_DRIFT")
                error = self.assert_unknown(world.guard, inv)
            finally:
                world.probe_error = None
                inv.close()
        self.assertEqual(error.diagnostic, {"stage": "READER_GUARD", "inventory": "PAPER_INVENTORY_PROBE"})
        self.assertFalse(world.floor())


class ReplayTests(InventoryCase):
    def proved(self, **kwargs):
        world = self.world()
        stack = contextlib.ExitStack()
        self.addCleanup(stack.close)
        stack.enter_context(world.seams())
        inv = world.prove(**kwargs)
        self.addCleanup(inv.close)
        return world, inv

    def test_consume_is_single_use(self):
        world, inv = self.proved()
        self.assertEqual(world.guard(inv), "LEGACY_OFF")
        error = self.assert_unknown(world.guard, inv)
        self.assertEqual(error.diagnostic, {"stage": "READER_GUARD", "inventory": "PAPER_INVENTORY_REPLAY"})
        self.assertEqual(len(world.receipts()), 1)

    def test_binding_mismatch_is_replay(self):
        other = "c3" * 20
        cases = {
            "other_root": lambda w: ("/opt/other", "/opt/other/releases/" + COMMIT, COMMIT),
            "other_commit": lambda w: (str(w.install), str(w.install / "releases" / other), other),
            "other_target": lambda w: (str(w.install), str(w.install / "releases" / SOURCE), COMMIT),
        }
        for name, arguments in cases.items():
            with self.subTest(case=name):
                world, inv = self.proved()
                self.assert_inventory_error("PAPER_INVENTORY_REPLAY", inv.consume, *arguments(world))
                # A failed consumption still spends the handle.
                self.assert_inventory_error("PAPER_INVENTORY_REPLAY", inv.consume,
                                            str(world.install), str(world.target), COMMIT)

    def test_staging_handle_is_never_consumable(self):
        world = self.world()
        stage = world.install / "releases" / (".staging." + COMMIT + ".fixture")
        with world.seams():
            inv = inventory.prove(str(world.install), str(world.target), COMMIT, world.lock_fd,
                                  staging=True)
            try:
                error = self.assert_unknown(world.guard, inv)
            finally:
                inv.close()
            self.assertEqual(error.diagnostic, {"stage": "READER_GUARD", "inventory": "PAPER_INVENTORY_REPLAY"})
            os.rename(world.target, stage)
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                result = release.main(["history-preflight", "--install-root", str(world.install),
                                       "--target-release", str(stage), "--commit", COMMIT,
                                       "--lock-fd", str(world.lock_fd)])
            self.assertEqual((result, stdout.getvalue()), (0, "PAPER_HISTORY_PREFLIGHT: ABSENT\n"))
        self.assertEqual(world.receipts(), [])
        self.assertFalse(world.floor())
        self.assertFalse((world.install / inventory.RECEIPT_DIRECTORY).exists())

    def test_different_lock_identity_is_unknown(self):
        world, inv = self.proved()
        lock = world.install / ".lagrange-release.lock"
        replacement = world.install / ".lagrange-release.lock.new"
        replacement.write_bytes(b"")
        os.chmod(replacement, 0o600)
        os.rename(replacement, lock)
        self.assert_unknown(world.guard, inv)
        self.assertEqual(world.receipts(), [])

    def test_unlocked_caller_descriptor_is_rejected(self):
        world = self.world()
        other = os.open(world.install / ".lagrange-release.lock", os.O_RDWR | os.O_CLOEXEC)
        self.addCleanup(os.close, other)
        with world.seams():
            self.assert_inventory_error("PAPER_INVENTORY_LOCK", inventory.prove, str(world.install),
                                        str(world.target), COMMIT, other)
            for value in (2, "5", None, True, 3.0):
                self.assert_inventory_error("PAPER_INVENTORY_LOCK", inventory.prove, str(world.install),
                                            str(world.target), COMMIT, value)

    def test_age_and_boot_are_bound(self):
        world, inv = self.proved()
        with mock.patch.object(inventory, "_monotonic_ns",
                               return_value=inv.proved_monotonic_ns + inventory.MAX_AGE_NS + 1):
            error = self.assert_unknown(world.guard, inv)
        self.assertEqual(error.diagnostic, {"stage": "READER_GUARD", "inventory": "PAPER_INVENTORY_STALE"})
        world2, inv2 = self.proved()
        with mock.patch.object(inventory, "_boot_id", return_value="00000000-0000-0000-0000-000000000000"):
            error = self.assert_unknown(world2.guard, inv2)
        self.assertEqual(error.diagnostic, {"stage": "READER_GUARD", "inventory": "PAPER_INVENTORY_STALE"})

    def test_second_module_copy_and_mixed_arguments_are_unknown(self):
        world, inv = self.proved()
        path = HERE / "lib/owner-beta-paper-history-inventory.py"
        spec = importlib.util.spec_from_file_location("paper_history_inventory_second_copy", path)
        second = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(second)
        fields = {name: object.__getattribute__(inv, name) for name in inventory.PaperHistoryInventory.__slots__}
        forged = second.PaperHistoryInventory(second._MINT, **fields)
        object.__setattr__(forged, "_lock_fd", os.dup(world.lock_fd))
        self.addCleanup(forged.close)
        self.assert_unknown(world.guard, forged)
        with self.assertRaises(second.PaperHistoryInventoryError):
            second.prove(str(world.install), str(world.target), COMMIT, world.lock_fd)
        for history_present in (False, True):
            self.assert_unknown(release.reader_guard, str(world.install), str(world.target), COMMIT,
                                history_present, inventory=inv)
        for value in (object(), "ABSENT", True, mock.Mock(spec=inventory.PaperHistoryInventory)):
            self.assert_unknown(world.guard, value)
        self.assertFalse(object.__getattribute__(inv, "_consumed"))
        self.assertEqual(world.guard(inv), "LEGACY_OFF")

    def test_handle_is_private_immutable_and_uncopyable(self):
        world, inv = self.proved()
        with self.assertRaises(TypeError):
            inventory.PaperHistoryInventory(object(), outcome="ABSENT")
        with self.assertRaises(AttributeError):
            inv.outcome = "PRESENT"
        with self.assertRaises(AttributeError):
            inv._consumed = False
        for clone in (copy.copy, copy.deepcopy, pickle.dumps):
            with self.assertRaises(TypeError):
                clone(inv)
        receipt = inventory.write_receipt
        self.assert_inventory_error("PAPER_INVENTORY_RECEIPT", receipt, inv, "LEGACY_OFF")
        self.assertEqual(world.receipts(), [])
        inv.close()
        inv.close()
        self.assert_inventory_error("PAPER_INVENTORY_CLOSED", inv.revalidate)
        self.assert_unknown(world.guard, inv)


class CliTests(InventoryCase):
    def run_cli(self, *arguments):
        stderr, stdout = io.StringIO(), io.StringIO()
        with contextlib.redirect_stderr(stderr), contextlib.redirect_stdout(stdout):
            result = release.main(list(arguments))
        return result, stdout.getvalue(), stderr.getvalue()

    def test_argument_misuse_exits_one(self):
        world = self.world()
        common = ["--install-root", str(world.install), "--target-release", str(world.target),
                  "--commit", COMMIT]
        fd = str(world.lock_fd)
        cases = (
            ["reader-guard", *common, "--history-inventory"],
            ["reader-guard", *common, "--history-inventory", "--lock-fd", "2"],
            ["reader-guard", *common, "--history-inventory", "--lock-fd", "-3"],
            ["reader-guard", *common, "--history-inventory", "--lock-fd", "x"],
            ["reader-guard", *common, "--history-present", "--history-inventory", "--lock-fd", fd],
            ["reader-guard", *common, "--lock-fd", fd],
            ["reader-guard", *common, "--history-present", "--lock-fd", fd],
            ["history-preflight", *common],
            ["history-preflight", *common, "--lock-fd", "2"],
        )
        with world.seams(), mock.patch.object(inventory, "prove") as prove:
            for arguments in cases:
                with self.subTest(arguments=arguments):
                    result, stdout, stderr = self.run_cli(*arguments)
                    self.assertEqual((result, stdout), (1, ""))
                    self.assertEqual(stderr.splitlines()[0], "PAPER_RELEASE_HISTORY_UNKNOWN")
            prove.assert_not_called()
        self.assertFalse(world.floor())

    def test_bare_cli_on_fixture_root_still_fails(self):
        world = self.world()
        with world.seams():
            result, stdout, stderr = self.run_cli(
                "reader-guard", "--install-root", str(world.install), "--target-release",
                str(world.target), "--commit", COMMIT)
        self.assertEqual((result, stdout, stderr), (1, "", "PAPER_RELEASE_HISTORY_UNKNOWN\n"))
        self.assertFalse(world.floor())


class StaticTests(unittest.TestCase):
    def test_sql_file_is_static_ascii_with_closed_grammar_and_exact_tables(self):
        raw = (ROOT / inventory.SQL_RELATIVE).read_bytes()
        text = raw.decode("ascii")
        self.assertTrue(text.endswith("ROLLBACK;\n"))
        self.assertIn("BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;", text)
        self.assertIn("SELECT 'PAPER_HISTORY_INVENTORY: '", text)
        self.assertNotRegex(text, r"(?im)^\s*(INSERT|UPDATE|DELETE|CREATE|DROP|ALTER|GRANT|TRUNCATE|COMMIT)\b")
        created = set()
        for path in sorted((ROOT / "migrations").glob("006[6-9]_*.up.sql")):
            created |= set(re.findall(r"(?m)^CREATE TABLE public\.(owner_beta_paper_[a-z_]+)", path.read_text()))
        self.assertEqual(len(created), 12)
        listed = set(re.findall(r"'(owner_beta_paper_[a-z_]+)'", text))
        self.assertEqual(listed, created)
        for earlier in sorted((ROOT / "migrations").glob("*.up.sql")):
            if int(earlier.name[:4]) <= 65:
                # Every predicate class must be empty on a real 0065 database.
                self.assertNotRegex(
                    earlier.read_text(),
                    r"owner_beta_paper|paper_006[6-9]_|paper_preview_beta_|_owner_beta_key|"
                    r"_owner_account_beta_key|_exact_beta_target_key|_beta_lineage_fkey|"
                    r"paper_preview_input_kind_check|beta_lineage_id|beta_apply_mode|beta_request_key|"
                    r"account_state_preimage|signal_allocation_sha256")

    def test_expected_lines_follow_sqlx_checksums_and_grammar(self):
        files = sorted((ROOT / "migrations").glob("*.up.sql"))
        self.assertEqual([version for version, _ in CHECKSUMS], list(range(1, len(files) + 1)))
        for (version, digest), path in zip(CHECKSUMS, files):
            self.assertEqual((version, digest), (int(path.name[:4]), hashlib.sha384(path.read_bytes()).hexdigest()))
        self.assertIn(PRE_LINE, EXPECTED)
        self.assertIn(SCHEMA_LINE, EXPECTED)
        self.assertEqual(len(EXPECTED), 65 + len(CHECKSUMS) - 68)
        for line in EXPECTED:
            self.assertRegex(line, inventory.LINE)
        self.assertNotIn(line_for("PRE_PAPER", 66), EXPECTED)
        self.assertNotIn(line_for("PAPER_SCHEMA", 68), EXPECTED)

    def test_codes_and_diagnostics_are_closed(self):
        self.assertEqual(release._INVENTORY_CODES, inventory.CODES)
        for code in sorted(inventory.CODES):
            for stage in ("READER_GUARD", "HISTORY_PREFLIGHT"):
                value = {"stage": stage, "inventory": code}
                self.assertEqual(release._diagnostic(value), value)
        for value in ({"stage": "READER_GUARD", "inventory": "SECRET_SENTINEL"},
                      {"stage": "STAGE", "inventory": "PAPER_INVENTORY_SQL"},
                      {"stage": "READER_GUARD", "inventory": "PAPER_INVENTORY_SQL", "x": 1},
                      {"stage": "READER_GUARD", "daily_guard": "PAPER_INVENTORY_SQL"}):
            self.assertIsNone(release._diagnostic(value))
        self.assertEqual(inventory.PaperHistoryInventoryError("SECRET_SENTINEL").code,
                         "PAPER_INVENTORY_PRIVATE")
        self.assertIs(sys.modules[inventory.MODULE_NAME], inventory)
        self.assertIs(release._history_inventory(), inventory)


# Every production/test entry point resolves one inventory module object.  Run
# in a clean interpreter so no earlier test has pre-registered the fixed name.
_IMPORT_ORDER_PROBE = r'''
import importlib.util, sys
lib, order = sys.argv[1], sys.argv[2].split(",")
def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, lib + "/" + filename)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module
copies = []
for step in order:
    if step == "cli":            # owner-beta-paper-release.py run as the deploy CLI
        copies.append(load("paper_release_cli_copy", "owner-beta-paper-release.py"))
    elif step == "ops":          # Paper operations loads its own release copy
        ops = load("owner_beta_paper_operations", "owner-beta-paper-operations.py")
        copies.append(sys.modules["owner_beta_paper_operations_release"])
    elif step == "daily":        # Daily activation_binding's load_local copy
        daily = load("daily_operations_order_probe", "owner-beta-daily-operations.py")
        copies.append(daily.load_local("daily_paper_release_binding", "owner-beta-paper-release.py"))
    elif step == "direct":       # a caller resolving the module first
        copies.append(None)
modules = []
for copy in copies:
    if copy is None:
        modules.append(sys.modules.get("owner_beta_paper_history_inventory"))
    else:
        modules.append(copy._history_inventory())
first = modules[0] if modules[0] is not None else modules[-1]
assert first is not None and all(m is None or m is first for m in modules), "two inventory copies"
assert sys.modules["owner_beta_paper_history_inventory"] is first
assert first.__name__ == first.MODULE_NAME == "owner_beta_paper_history_inventory"
releases = [c for c in copies if c is not None]
assert len({id(c) for c in releases}) == len(releases), "release copies must be distinct"
for copy in releases:
    assert getattr(sys.modules[copy._INVENTORY_NAME], "PaperHistoryInventory") is first.PaperHistoryInventory
print("ONE_INVENTORY_MODULE " + str(len(releases)))
'''


class ModuleIdentityTests(unittest.TestCase):
    def test_every_import_order_resolves_one_inventory_module(self):
        orders = ("cli,daily,ops", "daily,cli,ops", "ops,daily,cli", "daily,ops", "ops,cli",
                  "direct,daily,cli")
        for order in orders:
            with self.subTest(order=order):
                result = subprocess.run(
                    [sys.executable, "-I", "-B", "-c", _IMPORT_ORDER_PROBE, str(HERE / "lib"), order],
                    capture_output=True, timeout=60, cwd=str(HERE))
                self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace")[-2000:])
                self.assertRegex(result.stdout.decode(), r"\AONE_INVENTORY_MODULE [0-9]\n\Z")


def _load_daily_operations(name):
    path = HERE / "lib/owner-beta-daily-operations.py"
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class DailyActivationTests(InventoryCase):
    """Daily activation_binding drives the real prove/guard/close path.

    Daily loads its own release copy (`daily_paper_release_binding`) through
    its real `load_local`; the inventory module stays the single fixed-name
    object, so the handle minted there passes the guard's exact-type check.
    """

    VALUES = {
        "OWNER_BETA_DAILY_MODE": "owner_only", "OWNER_BETA_ACCESS_MODE": "owner_only",
        "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1", "OWNER_BETA_PAPER_MODE": "disabled",
        "LAGRANGE_DATA_DIR": "/data/daily", "LAGRANGE_ARTIFACTS_DIR": "/data/artifacts",
        "LAGRANGE_RUNTIME_SECRET_DIR": "/run/secrets", "POSTGRES_DB": "lagrange",
        "MAX_JOBS_PER_OWNER": "10", "PRICE_BETA_LEASE_SECS": "60", "PRICE_BETA_BACKOFF_SECS": "30",
    }

    @classmethod
    def setUpClass(cls):
        cls.daily_ops = _load_daily_operations("daily_operations_history_activation_test")

    def bind(self, world, **kwargs):
        with mock.patch.object(self.daily_ops.install, "verify_config",
                               return_value=({}, "sha256:" + "1" * 64)):
            return self.daily_ops.activation_binding(
                str(world.target), COMMIT, PROJECT, {"owner-beta-runner": "sha256:" + "2" * 64},
                dict(self.VALUES), **kwargs)

    @staticmethod
    def open_fds():
        return set(os.listdir("/proc/self/fd"))

    def test_marker_less_root_proves_absence_under_the_held_lock(self):
        world = self.world()
        before = self.open_fds()
        first = datetime.datetime(2026, 11, 17, 1, 2, 3, tzinfo=datetime.timezone.utc)
        with world.seams():
            # main() and Operations.prepare/enable each bind once per process.
            for now in (first, first + datetime.timedelta(seconds=1)):
                with mock.patch.object(inventory, "_utc_now", return_value=now):
                    binding, _config = self.bind(world, lock_fd=world.lock_fd)
                    self.assertEqual(binding.commit, COMMIT)
        self.assertTrue(world.execute_calls)
        self.assertEqual(len(world.receipts()), 2)
        self.assertFalse(world.floor())
        self.assertIs(sys.modules[inventory.MODULE_NAME], inventory)
        daily_release = sys.modules["daily_paper_release_binding"]
        self.assertIsNot(daily_release, release)
        self.assertIs(daily_release._history_inventory(), inventory)
        # Every retained descriptor was closed; the caller's lock is still held.
        self.assertEqual(self.open_fds(), before)
        fcntl.flock(world.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def test_inventory_failures_map_to_the_fixed_release_error(self):
        world = self.world()
        before = self.open_fds()
        world.sql_error = RuntimeError("SECRET_SENTINEL provider text")
        with world.seams():
            with self.assertRaises(Exception) as caught:
                self.bind(world, lock_fd=world.lock_fd)
        error = caught.exception
        self.assertEqual(type(error).__name__, "PaperReleaseError")
        self.assertIs(type(error), sys.modules["daily_paper_release_binding"].PaperReleaseError)
        self.assertEqual(error.code, "PAPER_RELEASE_HISTORY_UNKNOWN")
        self.assertEqual(error.diagnostic, {"stage": "READER_GUARD", "inventory": "PAPER_INVENTORY_SQL"})
        self.assertNotIn("SECRET_SENTINEL", repr(error) + str(error.args))
        self.assertIsNone(error.__cause__)
        self.assertEqual(world.receipts(), [])
        self.assertFalse(world.floor())
        self.assertEqual(self.open_fds(), before)

    def test_same_second_second_receipt_fails_closed(self):
        world = self.world()
        now = datetime.datetime(2026, 11, 17, 1, 2, 3, tzinfo=datetime.timezone.utc)
        with world.seams(), mock.patch.object(inventory, "_utc_now", return_value=now):
            self.bind(world, lock_fd=world.lock_fd)
            with self.assertRaises(Exception) as caught:
                self.bind(world, lock_fd=world.lock_fd)
        self.assertEqual(caught.exception.code, "PAPER_RELEASE_HISTORY_UNKNOWN")
        self.assertEqual(caught.exception.diagnostic,
                         {"stage": "READER_GUARD", "inventory": "PAPER_INVENTORY_RECEIPT"})
        self.assertEqual(len(world.receipts()), 1)

    def test_marker_root_never_mints_a_proof(self):
        world = self.world()
        (world.install / release.FLOOR_NAME).write_bytes(b"{}")
        with world.seams(), mock.patch.object(inventory, "prove",
                                              side_effect=AssertionError("must not prove")) as prove:
            with contextlib.suppress(Exception):
                self.bind(world, lock_fd=world.lock_fd)
        prove.assert_not_called()
        self.assertEqual(world.execute_calls, [])
        self.assertEqual(world.receipts(), [])

    def test_without_lock_the_marker_less_root_stays_unknown(self):
        world = self.world()
        with world.seams(), mock.patch.object(inventory, "prove",
                                              side_effect=AssertionError("must not prove")) as prove:
            with self.assertRaises(Exception) as caught:
                self.bind(world)
        prove.assert_not_called()
        self.assertEqual(caught.exception.code, "PAPER_RELEASE_HISTORY_UNKNOWN")
        self.assertEqual(world.receipts(), [])
        self.assertFalse(world.floor())

    def test_handle_from_another_release_copy_is_accepted_by_daily_guard(self):
        world = self.world()
        with world.seams():
            self.bind(world, lock_fd=world.lock_fd)  # loads the Daily release copy
            daily_release = sys.modules["daily_paper_release_binding"]
            self.assertIsNot(daily_release, release)
            # Minted through the test's release copy, consumed by Daily's copy.
            inv = world.prove()
            try:
                with mock.patch.object(inventory, "_utc_now", return_value=datetime.datetime(
                        2030, 1, 1, tzinfo=datetime.timezone.utc)):
                    self.assertEqual(daily_release.reader_guard(
                        str(world.install), str(world.target), COMMIT, None, True, inventory=inv),
                        "LEGACY_OFF")
            finally:
                inv.close()


if __name__ == "__main__":
    unittest.main()
