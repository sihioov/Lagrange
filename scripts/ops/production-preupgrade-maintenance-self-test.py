#!/usr/bin/env python3
"""Private filesystem/OpenSSL maintenance tests; Docker/systemd observations are fakes.

These do not claim a production writer fence or a native PostgreSQL restore.
The separate Docker suite exercises the real cold-cluster restore backend.
"""
from __future__ import annotations

import copy
import datetime
import fcntl
import importlib.util
import io
import itertools
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts/ops" / file)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


maintenance = load("preupgrade_maintenance_tested", "prepare-production-recovery.py")
fixture = load("preupgrade_maintenance_fixture", "production-preupgrade-recovery-self-test.py")
files = maintenance.files
ERROR = maintenance.runtime.RuntimeErrorCode


def unit_value(name):
    unit = {"category": "stopped", "LoadState": "loaded", "ActiveState": "inactive", "SubState": "dead",
        "FragmentPath": "/etc/systemd/system/" + name, "DropInPaths": "", "NeedDaemonReload": "no",
        "UnitFileState": "disabled", "ActiveEnterTimestampMonotonic": "0",
        "InactiveEnterTimestampMonotonic": "1", "InvocationID": "", "Transient": "no", "Job": "",
        "Triggers": "", "TriggeredBy": "", "OnFailure": "", "OnSuccess": "",
        "OnFailureOf": "", "OnSuccessOf": ""}
    if name.endswith(".service"):
        unit.update(ExecMainStartTimestampMonotonic="0", ExecMainExitTimestampMonotonic="0",
                    MainPID="0", ControlPID="0", Restart="no", NRestarts="0", ControlGroup="",
                    Slice="system.slice")
    return unit


def ancestry_value(path, *, actual=False, mode=0o750):
    paths = [Path("/"), *reversed(Path(path).parents[:-1]), Path(path)]
    if actual:
        return [list(files.ancestor_identity(p.stat())) for p in paths]
    return [[1, i + 1, 999 if i == len(paths) - 1 else 0,
             999 if i == len(paths) - 1 else 0,
             mode if i == len(paths) - 1 else 0o755] for i in range(len(paths))]


def value():
    now = maintenance.utc().replace(microsecond=0)
    old, tool = "a" * 40, "b" * 40
    roots = [{"name": name, "path": path, "uid": uid, "gid": uid, "mode": 0o750, "parent": None}
        for name, path, uid in (
            ("raw", "/var/lib/lagrange/data/raw", 10001),
            ("curated", "/var/lib/lagrange/data/curated", 10001),
            ("base", "/var/lib/lagrange/artifacts/historical-price-beta-root", 10001),
            ("config", "/opt/lagrange/releases/" + old, 0),
            ("postgres", "/var/lib/docker/volumes/fixture/_data", 999))]
    units = {name: unit_value(name) for name in maintenance.transition.UNITS}
    old_root, tool_root = roots[3]["path"], "/opt/lagrange/maintenance/" + tool
    paths = [tool_root + "/" + p for p in maintenance.PAYLOAD] + [old_root + p for p in
        ("/.lagrange-release", "/.lagrange-release-manifest", "/deploy/compose/.env", "/deploy/compose/compose.yml")]
    paths += ["/etc/lagrange/production-backup.conf", "/opt/lagrange/bin/run-production-backup.sh"]
    paths += [u["FragmentPath"] for u in units.values()]
    return {"schema": maintenance.SCHEMA, "recovery_id": "c" * 32, "old_commit": old, "tool_commit": tool,
        "issued_at": (now - datetime.timedelta(seconds=5)).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "expires_at": (now + datetime.timedelta(minutes=20)).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "boot_id": "12345678-1234-1234-1234-123456789abc", "project": "lagrange", "database": "lagrange",
        "postgres": {"id": "d" * 64, "image_id": "sha256:" + "e" * 64,
            "created": "2026-10-01T00:00:00Z", "started_at": "2026-10-01T00:01:00Z", "restart_count": 0,
            "volume": "fixture", "mountpoint": roots[4]["path"], "volume_created_at": "2026-10-01T00:00:00Z",
            "networks": {"lagrange_backend": "f" * 64}},
        "postgres_ancestry": ancestry_value(roots[4]["path"]),
        "unit_graph": "reciprocal", "roots": roots, "writers": [], "units": units,
        "pins": {p: "sha256:" + "0" * 64 for p in paths},
        "migrations": fixture.PreupgradeRecoveryTests.audit()["migrations"],
        "base_manifest_sha256": "sha256:" + "1" * 64, "secret_recovery_ref": "fixture-recovery-reference",
        "max_total_bytes": 1024**2, "min_free_bytes": 1024**3}


class FakeNative:
    def __init__(self):
        self.observations = []
        self.mutation = None

    def guard(self, stopped):
        self.observations.append(stopped)
        if self.mutation:
            self.mutation(len(self.observations))
        return "2026-10-16T09:00:00Z" if stopped else None


class MaintenanceTests(unittest.TestCase):
    put = fixture.PreupgradeRecoveryTests.put

    def setUp(self):
        fixture.PreupgradeRecoveryTests.setUp(self)
        self.value = value()
        self.value["roots"] = [{"name": r.name, "path": r.path, "uid": r.uid, "gid": r.gid,
                                "mode": r.mode, "parent": None} for r in self.roots]
        self.value["postgres_ancestry"] = ancestry_value(self.roots[-1].path, actual=True)
        with files.Snapshot(maintenance.root_values(self.value), self.limits, profile="preupgrade") as snapshot:
            self.value["base_manifest_sha256"] = maintenance.digest(files.canonical(
                [r for r in snapshot.records if r["path"] == "base" or r["path"].startswith("base/")]))
        root = self.directory / "backups"
        root.mkdir(mode=0o700)
        self.key = self.put("key", b"a" * 64, mode=0o600)
        self.storage = maintenance.Storage(self.value, None, maintenance.Window(self.value))
        self.storage.root = str(root)
        self.storage.values["BACKUP_ROOT"] = str(root)
        self.storage.values["KEY_FILE"] = str(self.key)
        self.storage.verify_cluster = mock.Mock(return_value=fixture.PreupgradeRecoveryTests.audit())
        self.native = FakeNative()
        self.receipt = {"schema": maintenance.SCHEMA,
            "binding_sha256": maintenance.digest(files.canonical(maintenance.binding(self.value))),
            "boot_id": self.value["boot_id"], "observed_at": maintenance.utc().strftime("%Y-%m-%dT%H:%M:%SZ"),
            "database": fixture.PreupgradeRecoveryTests.audit()}
        self.audit_path = root / ("preupgrade-audit-" + self.value["recovery_id"] + ".json")
        maintenance.write_json(root, self.audit_path.name, self.receipt)
        self.final = root / ("preupgrade-" + self.value["recovery_id"])

    def assert_no_staging(self):
        self.assertFalse(any(p.name.startswith(".preupgrade-") for p in Path(self.storage.root).iterdir()))

    def protected_verifier_sources(self):
        release = self.directory / "maintenance"
        directory = release / "deploy/db"
        directory.mkdir(parents=True, mode=0o700)
        for name in ("preupgrade-recovery-verify.sh", "preupgrade-recovery-audit.sql"):
            path = directory / name
            path.write_bytes((ROOT / "deploy/db" / name).read_bytes())
            path.chmod(0o600)
        self.storage.context = types.SimpleNamespace(release=str(release))
        return directory

    def test_private_verifier_copies_preserve_root_only_installed_sources(self):
        source = self.protected_verifier_sources()
        before = {p.name: (p.read_bytes(), maintenance.install.file_identity(p.stat()))
                  for p in source.iterdir()}
        with self.storage.temporary(".preupgrade-scripts-") as stage:
            paths = self.storage.verifier_scripts(stage)
            for name, path in zip(("preupgrade-recovery-verify.sh", "preupgrade-recovery-audit.sql"), paths):
                copied = Path(path)
                self.assertEqual(copied.parent, stage)
                self.assertEqual(copied.read_bytes(), before[name][0])
                self.assertEqual((copied.stat().st_uid, copied.stat().st_gid,
                                  stat.S_IMODE(copied.stat().st_mode)), (0, 0, 0o444))
                self.assertNotEqual(copied.stat().st_ino, (source / name).stat().st_ino)
            self.assertEqual(stat.S_IMODE(stage.stat().st_mode), 0o700)
        self.assertEqual(before, {p.name: (p.read_bytes(), maintenance.install.file_identity(p.stat()))
                                  for p in source.iterdir()})
        self.assert_no_staging()

    def test_private_verifier_copies_refuse_existing_destination(self):
        self.protected_verifier_sources()
        with self.storage.temporary(".preupgrade-scripts-") as stage:
            existing = stage / "verify.sh"
            existing.write_bytes(b"retained")
            with self.assertRaises(FileExistsError):
                self.storage.verifier_scripts(stage)
            self.assertEqual(existing.read_bytes(), b"retained")
            self.assertFalse((stage / "verify.sql").exists())

    def test_private_verifier_copies_refuse_source_symlink_and_writable_source(self):
        source = self.protected_verifier_sources()
        script = source / "preupgrade-recovery-verify.sh"
        saved = script.read_bytes()
        script.unlink()
        script.symlink_to(source / "preupgrade-recovery-audit.sql")
        with self.storage.temporary(".preupgrade-scripts-") as stage:
            with self.assertRaises(OSError):
                self.storage.verifier_scripts(stage)
            self.assertEqual(list(stage.iterdir()), [])
        script.unlink()
        script.write_bytes(saved)
        script.chmod(0o666)
        with self.storage.temporary(".preupgrade-scripts-") as stage:
            with self.assertRaises(maintenance.install.DailyInstallError):
                self.storage.verifier_scripts(stage)
            self.assertEqual(list(stage.iterdir()), [])

    def test_private_verifier_copies_refuse_outside_or_shared_staging(self):
        self.protected_verifier_sources()
        with self.assertRaises(ERROR):
            self.storage.verifier_scripts(self.directory)
        with self.storage.temporary(".preupgrade-scripts-") as stage:
            stage.chmod(0o755)
            with self.assertRaises(maintenance.install.DailyInstallError):
                self.storage.verifier_scripts(stage)
            self.assertEqual(list(stage.iterdir()), [])

    def test_valid_contract_and_renewed_verify_keep_same_recovery_binding(self):
        original = value()
        self.assertEqual(maintenance.contract(files.canonical(original)), original)
        renewed = copy.deepcopy(original)
        renewed["boot_id"] = "87654321-1234-1234-1234-123456789abc"
        renewed["expires_at"] = (maintenance.utc() + datetime.timedelta(minutes=21)).strftime("%Y-%m-%dT%H:%M:%SZ")
        maintenance.contract(files.canonical(renewed))
        self.assertEqual(maintenance.binding(original), maintenance.binding(renewed))
        renewed["old_commit"] = "c" * 40
        self.assertNotEqual(maintenance.binding(original), maintenance.binding(renewed))

    def test_contract_refuses_expiry_unknown_fields_and_missing_payload_pin(self):
        cases = []
        expired = value()
        expired["expires_at"] = expired["issued_at"]
        cases.append(expired)
        unknown = value()
        unknown["shell_command"] = "fixture"
        cases.append(unknown)
        for relative in maintenance.PAYLOAD:
            missing = value()
            del missing["pins"]["/opt/lagrange/maintenance/" + missing["tool_commit"] + "/" + relative]
            cases.append(missing)
        for candidate in cases:
            with self.subTest(candidate=list(candidate)), self.assertRaises(ERROR):
                maintenance.contract(files.canonical(candidate))

    def test_contract_refuses_root_overlap_key_capture_and_incomplete_migration_prefix(self):
        for alter in (lambda v: v["roots"][2].update(path=v["roots"][0]["path"] + "/base"),
                      lambda v: v["roots"][2].update(path="/etc/lagrange/secrets"),
                      lambda v: v["migrations"][0].update(version=2)):
            candidate = value()
            alter(candidate)
            with self.assertRaises(ERROR):
                maintenance.contract(files.canonical(candidate))

    def test_contract_refuses_active_unit_missing_backup_timer_and_dropin(self):
        name = maintenance.transition.UNITS[0]
        for alter in (lambda v: v["units"][name].update(ActiveState="active"),
                      lambda v: v["units"].pop(maintenance.transition.UNITS[-1]),
                      lambda v: v["units"][name].update(DropInPaths="/unexpected.conf")):
            candidate = value()
            alter(candidate)
            with self.assertRaises(ERROR):
                maintenance.contract(files.canonical(candidate))

    def test_monotonic_deadline_cannot_be_extended_by_wall_clock_rollback(self):
        window = maintenance.Window(value())
        with mock.patch.object(maintenance.time, "monotonic", return_value=window.deadline + 1):
            with self.assertRaises(ERROR):
                window.remaining()

    def test_audit_boot_or_contract_or_time_change_prevents_capture(self):
        for key, change in (("boot_id", "different"), ("binding_sha256", "sha256:" + "0" * 64),
                            ("observed_at", "2000-01-01T00:00:00Z")):
            receipt = copy.deepcopy(self.receipt)
            receipt[key] = change
            with self.assertRaises(ERROR):
                maintenance.checked_receipt(receipt, self.value, fresh=True)
        self.assertFalse(self.final.exists())

    def test_audit_migration_checksum_mismatch_is_rejected_even_for_old_verify(self):
        receipt = copy.deepcopy(self.receipt)
        receipt["database"]["migrations"][0]["checksum"] = "a" * 96
        with self.assertRaises(ERROR):
            maintenance.checked_receipt(receipt, self.value, fresh=False)

    def test_audit_uses_fixed_read_only_command_and_never_overwrites_a_receipt(self):
        self.audit_path.unlink()
        calls = []
        def command(args, target, source=None, seconds=900):
            calls.append((args, source.read(), seconds))
            target.write_bytes(files.canonical(self.receipt["database"]) + b"\n")
            target.chmod(0o600)
        with mock.patch.object(self.storage, "command_file", side_effect=command):
            maintenance.audit(self.value, self.storage, self.native)
            original = self.audit_path.read_bytes()
            with self.assertRaises(FileExistsError):
                maintenance.audit(self.value, self.storage, self.native)
        self.assertEqual(self.audit_path.read_bytes(), original)
        self.assertEqual(self.native.observations, [False, False, False, False])
        self.assertIn(self.value["postgres"]["id"], calls[0][0])
        self.assertIn("PGUSER=lagrange", calls[0][0])
        self.assertNotIn("PGUSER=postgres", calls[0][0])
        self.assertIn("PGOPTIONS=-c default_transaction_read_only=on -c statement_timeout=120000 -c lock_timeout=5000",
                      calls[0][0])
        self.assertEqual(calls[0][1], (ROOT / "deploy/db/preupgrade-recovery-audit.sql").read_bytes())
        self.assertIn(b"PREUPGRADE_DATABASE_WRITERS", calls[0][1])
        self.assert_no_staging()

    def test_changed_writer_after_audit_prevents_receipt_publication(self):
        self.audit_path.unlink()
        def command(args, target, source=None, seconds=900):
            target.write_bytes(files.canonical(self.receipt["database"]) + b"\n")
            target.chmod(0o600)
        def changed(call):
            if call == 2:
                raise ERROR("PREUPGRADE_WRITERS_CHANGED")
        self.native.mutation = changed
        with mock.patch.object(self.storage, "command_file", side_effect=command), self.assertRaises(ERROR):
            maintenance.audit(self.value, self.storage, self.native)
        self.assertFalse(self.audit_path.exists())
        self.assert_no_staging()

    def test_capture_encrypts_all_files_verifies_clone_then_publishes_without_plaintext(self):
        maintenance.capture(self.value, self.storage, self.native)
        self.assertEqual({p.name for p in self.final.iterdir()}, maintenance.MEMBERS)
        self.assertEqual(self.native.observations, [True, True, True])
        self.storage.verify_cluster.assert_called_once()
        copied, expected = self.storage.verify_cluster.call_args.args
        self.assertEqual(copied.name, "restored")
        self.assertNotEqual(str(copied / "postgres"), self.value["roots"][-1]["path"])
        self.assertEqual(expected["database"], self.receipt["database"])
        for member in self.final.iterdir():
            self.assertEqual(member.stat().st_mode & 0o777, 0o600)
            self.assertNotIn(b"SYNTHETIC_PROTECTED_CONFIG", member.read_bytes())
        state = self.storage.verify_set(self.final)
        self.assertEqual(state["audit"], self.receipt)
        self.assertEqual(self.storage.verify_cluster.call_count, 2)
        self.assert_no_staging()

    def test_existing_recovery_set_refuses_without_altering_old_bytes(self):
        maintenance.capture(self.value, self.storage, self.native)
        original = {p.name: p.read_bytes() for p in self.final.iterdir()}
        with self.assertRaises(ERROR):
            maintenance.capture(self.value, self.storage, self.native)
        self.assertEqual(original, {p.name: p.read_bytes() for p in self.final.iterdir()})
        self.assert_no_staging()

    def test_missing_audit_and_wrong_base_fail_before_publication(self):
        self.audit_path.unlink()
        with self.assertRaises(FileNotFoundError):
            maintenance.capture(self.value, self.storage, self.native)
        self.assertEqual(self.native.observations, [])
        maintenance.write_json(self.audit_path.parent, self.audit_path.name, self.receipt)
        self.put("base/changed", b"different-base", 10001, 10001)
        with self.assertRaises(ERROR):
            maintenance.capture(self.value, self.storage, self.native)
        self.assertFalse(self.final.exists())
        self.assert_no_staging()

    def test_restore_failure_keeps_sources_and_never_publishes(self):
        before = (self.directory / "raw/manifest").read_bytes()
        self.storage.verify_cluster.side_effect = ERROR("PREUPGRADE_RESTORE_AUDIT_MISMATCH")
        with self.assertRaises(ERROR):
            maintenance.capture(self.value, self.storage, self.native)
        self.assertEqual((self.directory / "raw/manifest").read_bytes(), before)
        self.assertFalse(self.final.exists())
        self.assert_no_staging()

    def test_ambiguous_create_retains_owned_intent_and_copied_mount_for_recovery(self):
        self.protected_verifier_sources()
        def docker_call(args, timeout=15):
            if args[0] == "create":
                raise ERROR("SIMULATED_CREATE_TIMEOUT")
            self.assertEqual(args[0], "ps")
            return ""
        self.storage.docker = types.SimpleNamespace(call=docker_call, object=lambda *args:
            {"Id": "sha256:" + "e" * 64, "RepoDigests": [maintenance.recovery.backup.POSTGRES], "Config": {}})
        with self.assertRaises(ERROR):
            with self.storage.temporary(".preupgrade-ambiguous-") as stage:
                stream = io.BytesIO()
                with files.Snapshot(maintenance.root_values(self.value), self.limits, profile="preupgrade") as snapshot:
                    snapshot.write(stream)
                restored = stage / "restored"
                restored.mkdir(mode=0o700)
                manifest = files.restore_archive(io.BytesIO(stream.getvalue()), self.limits,
                                                  str(restored), profile="preupgrade")
                maintenance.recovery.ColdStorage.verify_cluster(self.storage, restored,
                    {"files": manifest, "database": self.receipt["database"]})
        self.assertTrue((restored / maintenance.recovery.PGROOT / "global/pg_control").exists())
        intents = list(Path(self.storage.root).glob("preupgrade-verifier-*.json"))
        self.assertEqual(len(intents), 1)
        intent = maintenance.install.json_bytes(intents[0].read_bytes())
        self.assertEqual(intent["staging"], str(stage))
        self.assertEqual(intent["labels"]["io.lagrange.preupgrade-project"], "lagrange")
        self.assertFalse(self.final.exists())

    def test_writer_restart_before_publish_rejects_and_discards_private_stage(self):
        def restart(call):
            if call == 2:
                raise ERROR("PREUPGRADE_WRITERS_CHANGED")
        self.native.mutation = restart
        with self.assertRaises(ERROR):
            maintenance.capture(self.value, self.storage, self.native)
        self.assertFalse(self.final.exists())
        self.assert_no_staging()

    def test_expiry_after_clone_verification_prevents_publication(self):
        def expire(copied, expected):
            self.storage.window.deadline = maintenance.time.monotonic() - 1
        self.storage.verify_cluster.side_effect = expire
        with self.assertRaises(ERROR):
            maintenance.capture(self.value, self.storage, self.native)
        self.assertFalse(self.final.exists())
        self.assert_no_staging()

    def test_post_publication_failure_preserves_set_and_does_not_claim_success(self):
        def changed(call):
            if call == 3:
                raise ERROR("PREUPGRADE_WRITERS_CHANGED")
        self.native.mutation = changed
        with self.assertRaises(ERROR):
            maintenance.capture(self.value, self.storage, self.native)
        self.assertEqual({p.name for p in self.final.iterdir()}, maintenance.MEMBERS)
        self.assert_no_staging()

    def test_actual_source_mutation_during_verification_prevents_publication(self):
        def mutate(copied, expected):
            self.put("raw/manifest", b"changed", 10001, 10001)
        self.storage.verify_cluster.side_effect = mutate
        with self.assertRaises(maintenance.files.SnapshotError):
            maintenance.capture(self.value, self.storage, self.native)
        self.assertFalse(self.final.exists())
        self.assert_no_staging()

    def test_cipher_tampering_and_wrong_key_never_reach_clone(self):
        maintenance.capture(self.value, self.storage, self.native)
        cipher = self.final / "files.bundle.enc"
        original = cipher.read_bytes()
        cipher.write_bytes(original[:-1] + bytes([original[-1] ^ 1]))
        self.storage.verify_cluster.reset_mock()
        with self.assertRaises(ERROR):
            self.storage.verify_set(self.final)
        self.storage.verify_cluster.assert_not_called()
        cipher.write_bytes(original)
        self.key.write_bytes(b"b" * 64)
        with self.assertRaises(ERROR):
            self.storage.verify_set(self.final)
        self.storage.verify_cluster.assert_not_called()
        self.assert_no_staging()

    def test_missing_complete_or_extra_plaintext_member_is_rejected(self):
        maintenance.capture(self.value, self.storage, self.native)
        marker = self.final / "COMPLETE"
        marker.unlink()
        with self.assertRaises(ERROR):
            self.storage.verify_set(self.final)
        marker.write_bytes((maintenance.FORMAT + "\n").encode())
        marker.chmod(0o600)
        (self.final / "unexpected").write_bytes(b"fixture")
        with self.assertRaises(ERROR):
            self.storage.verify_set(self.final)

    def test_publish_no_replace_preserves_existing_destination_on_race(self):
        stage = self.directory / "stage"
        stage.mkdir(mode=0o700)
        self.final.mkdir(mode=0o700)
        sentinel = self.final / "sentinel"
        sentinel.write_bytes(b"do-not-overwrite")
        with self.assertRaises(OSError):
            maintenance.publish(stage, self.storage.root, self.final.name)
        self.assertEqual(sentinel.read_bytes(), b"do-not-overwrite")
        self.assertTrue(stage.exists())

    def test_key_identity_and_format_are_checked_through_scope_exit(self):
        with maintenance.key_fence(str(self.key)):
            pass
        with self.assertRaises(ERROR):
            with maintenance.key_fence(str(self.key)):
                replacement = self.put("replacement-key", b"a" * 64, mode=0o600)
                replacement.replace(self.key)
        self.key.write_bytes(b"wrong")
        with self.assertRaises(ERROR):
            with maintenance.key_fence(str(self.key)):
                self.fail("invalid key admitted")

    def raw_lock(self):
        path = self.directory / "raw/manifests/provider=fixture/market=kr"
        path.mkdir(parents=True, mode=0o750)
        for p in (path, path.parent, path.parent.parent):
            p.chmod(0o750)
            os.chown(p, 10001, 10001)
        return self.put("raw/manifests/provider=fixture/market=kr/commit.lock", b"", 10001, 10001, 0o640)

    def test_actual_raw_writer_lock_contention_refuses_without_waiting(self):
        path = self.raw_lock()
        with path.open("rb") as writer:
            fcntl.flock(writer, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError):
                with maintenance.raw_fence(maintenance.root_values(self.value)[0]):
                    self.fail("busy Raw lock admitted")
        with maintenance.raw_fence(maintenance.root_values(self.value)[0]):
            pass

    def test_replaced_raw_lock_refuses_at_scope_exit(self):
        path = self.raw_lock()
        with self.assertRaises(ERROR):
            with maintenance.raw_fence(maintenance.root_values(self.value)[0]):
                path.rename(path.with_name("old.lock"))
                self.put(str(path.relative_to(self.directory)), b"", 10001, 10001, 0o640)

    def test_writer_inventory_includes_foreign_mount_network_and_host_network(self):
        base = fixture.PreupgradeRecoveryTests.container()
        base["NetworkSettings"] = {"Networks": {}}
        base["HostConfig"] = {"NetworkMode": "none"}
        base["Mounts"] = []
        base["Config"]["Labels"] = {}
        for change in (lambda c: c["Mounts"].append(
                           {"Source": self.value["roots"][0]["path"], "Destination": "/data/raw"}),
                       lambda c: c["NetworkSettings"]["Networks"].update(shared={"NetworkID": "f" * 64}),
                       lambda c: c["HostConfig"].update(NetworkMode="host")):
            observed = copy.deepcopy(base)
            change(observed)
            docker = types.SimpleNamespace(call=lambda args: observed["Id"], object=lambda *args: observed)
            native = maintenance.Native(self.value, docker, maintenance.Window(self.value))
            self.value["writers"] = [maintenance.stopped_writer(observed)]
            native.writers()
            observed["State"]["Running"] = True
            with self.assertRaises(ERROR):
                native.writers()

    def test_writer_generation_and_removed_expected_writer_are_rejected(self):
        observed = fixture.PreupgradeRecoveryTests.container()
        observed["Config"]["Labels"] = {"com.docker.compose.project": "lagrange"}
        observed["HostConfig"], observed["NetworkSettings"] = {}, {"Networks": {}}
        self.value["writers"] = [maintenance.stopped_writer(observed)]
        docker = types.SimpleNamespace(call=lambda args: observed["Id"], object=lambda *args: observed)
        native = maintenance.Native(self.value, docker, maintenance.Window(self.value))
        native.writers()
        observed["RestartCount"] += 1
        with self.assertRaises(ERROR):
            native.writers()
        docker.call = lambda args: ""
        with self.assertRaises(ERROR):
            native.writers()

    def test_unit_restart_timestamp_change_is_rejected(self):
        candidate = value()
        native = maintenance.Native(candidate, None, maintenance.Window(candidate))
        def observe(args):
            if args[1] in {"list-unit-files", "list-units"}:
                return "\n".join(name + " inactive" for name in candidate["units"]).encode()
            unit = {k: v for k, v in candidate["units"][args[-1]].items() if k != "category"}
            unit["ActiveEnterTimestampMonotonic"] = "999"
            return "\n".join(k + "=" + v for k, v in unit.items()).encode()
        native.call = observe
        with self.assertRaises(ERROR):
            native.units()

    def test_plan_has_no_docker_systemd_or_source_observations(self):
        with mock.patch.object(maintenance.runtime, "Docker") as docker, \
                mock.patch.object(maintenance.subprocess, "run") as run, \
                mock.patch("sys.stdout", new_callable=io.StringIO):
            self.assertEqual(maintenance.main(["--mode", "plan"]), 0)
        docker.assert_not_called()
        run.assert_not_called()


class FailureEvidenceTests(unittest.TestCase):
    def test_only_known_codes_are_returned_without_exception_text(self):
        sentinel = "secret-sentinel-never-record"
        for error_type in (ERROR, files.SnapshotError, maintenance.install.DailyInstallError, RuntimeError):
            self.assertEqual(maintenance.failure_details(error_type("PREUPGRADE_UNITS_CHANGED"))["code"],
                             "PREUPGRADE_UNITS_CHANGED")
            for message in (sentinel, "PREUPGRADE_" + sentinel, "PREUPGRADE_UNITS_CHANGED\n" + sentinel):
                result = maintenance.failure_details(error_type(message))
                self.assertEqual(result["code"], "UNKNOWN")
                self.assertNotIn(sentinel, str(result))
        error = PermissionError(13, sentinel, "/private/" + sentinel)
        self.assertEqual(maintenance.failure_details(error)["code"], "OS_ERROR_13")
        self.assertNotIn(sentinel, str(maintenance.failure_details(error)))
        self.assertEqual(maintenance.failure_details(OSError(999, sentinel))["code"], "UNKNOWN")

    def test_locations_contain_only_owned_source_lines_and_never_locals(self):
        try:
            maintenance.checked_units({}, "reciprocal")
        except ERROR as error:
            result = maintenance.failure_details(error)
        self.assertEqual(result["code"], "PREUPGRADE_UNITS")
        self.assertTrue(result["locations"])
        self.assertLessEqual(len(result["locations"]), 4)
        for location in result["locations"]:
            self.assertEqual(set(location), {"source", "line"})
            self.assertIn(location["source"], maintenance.PAYLOAD)
            self.assertIs(type(location["line"]), int)
            self.assertGreater(location["line"], 0)

    def test_receipt_is_exclusive_and_preserves_first_failure(self):
        with tempfile.TemporaryDirectory(prefix="preupgrade-failure-", dir="/var/lib") as directory:
            real_write = maintenance.write_json
            def private_write(root, name, receipt):
                self.assertEqual(root, Path("/srv/backups/lagrange"))
                real_write(Path(directory), name, receipt)
            candidate = value()
            with mock.patch.object(maintenance, "write_json", side_effect=private_write):
                maintenance.record_failure(candidate, "audit", ERROR("PREUPGRADE_UNITS_CHANGED"))
                path = Path(directory) / ("preupgrade-failure-" + candidate["recovery_id"] + "-audit.json")
                before = path.read_bytes()
                self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
                receipt = maintenance.install.json_bytes(before)
                self.assertFalse(receipt["recoveryAccepted"])
                self.assertEqual(receipt["binding_sha256"],
                                 maintenance.digest(files.canonical(maintenance.binding(candidate))))
                self.assertEqual(receipt["error"]["code"], "PREUPGRADE_UNITS_CHANGED")
                maintenance.record_failure(candidate, "audit", ERROR("PREUPGRADE_VOLUME_CHANGED"))
                self.assertEqual(path.read_bytes(), before)
                self.assertEqual(len(list(Path(directory).iterdir())), 1)

    def test_receipt_capacity_failure_is_not_retried_and_unparsed_contract_writes_nothing(self):
        with mock.patch.object(maintenance, "write_json", side_effect=OSError(28, "private")) as write:
            maintenance.record_failure(None, "audit", ERROR("PREUPGRADE_UNITS"))
            write.assert_not_called()
            maintenance.record_failure(value(), "audit", ERROR("PREUPGRADE_UNITS"))
            write.assert_called_once()

    def test_main_failure_retains_failure_exit_and_emits_no_exception_body(self):
        candidate = value()
        error = ERROR("PREUPGRADE_UNITS_CHANGED")
        output = io.StringIO()
        with mock.patch.object(maintenance.install, "require_root"), \
                mock.patch.object(maintenance.os, "umask"), \
                mock.patch.object(maintenance.install, "read_root_file", return_value=b"fixture"), \
                mock.patch.object(maintenance, "contract", return_value=candidate), \
                mock.patch.object(maintenance, "ROOT",
                                  Path("/opt/lagrange/maintenance") / candidate["tool_commit"]), \
                mock.patch.object(maintenance, "Window", side_effect=error), \
                mock.patch.object(maintenance, "record_failure") as record, \
                mock.patch.object(maintenance.signal, "setitimer"), \
                mock.patch.object(sys, "stderr", output):
            self.assertEqual(maintenance.main(["--mode", "audit", "--contract", "/fixture"]), 1)
        record.assert_called_once_with(candidate, "audit", error)
        self.assertEqual(output.getvalue(),
                         "PREUPGRADE: FAILED_WRITERS_MUST_REMAIN_STOPPED_INSPECT_OWNED_RESOURCES\n")


class StagedPayloadTests(unittest.TestCase):
    def staged(self, missing=None):
        with tempfile.TemporaryDirectory(prefix="preupgrade-payload-") as directory:
            root = Path(directory)
            stage, outside = root / "stage", root / "outside"
            outside.mkdir()
            for relative in maintenance.PAYLOAD:
                if relative == missing:
                    continue
                target = stage / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes((ROOT / relative).read_bytes())
            before = {str(p.relative_to(root)): p.read_bytes() for p in root.rglob("*") if p.is_file()}
            result = subprocess.run(["/usr/bin/python3", "-I", "-B",
                str(stage / "scripts/ops/prepare-production-recovery.py"), "--mode", "plan"],
                cwd=outside, env={"PATH": "/usr/bin:/bin", "HOME": str(outside), "LC_ALL": "C"},
                stdin=subprocess.DEVNULL, capture_output=True, timeout=20, check=False)
            self.assertEqual(before, {str(p.relative_to(root)): p.read_bytes()
                                     for p in root.rglob("*") if p.is_file()})
            self.assertEqual(list(outside.iterdir()), [])
            return result

    def test_exact_staged_payload_plans_without_checkout_imports_or_writes(self):
        result = self.staged()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, b"")
        self.assertEqual(result.stdout, b"PREUPGRADE: PLAN fresh host contract; inactive writers; "
                         b"explicit DB stop; isolated clone; no restart\n")

    def test_missing_staged_base_dependency_cannot_plan(self):
        result = self.staged("scripts/ops/lib/owner-beta-daily-base.py")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn(b"PREUPGRADE: PLAN", result.stdout)
        self.assertIn(b"owner-beta-daily-base.py", result.stderr)


class UnitInventoryTests(unittest.TestCase):
    history = "lagrange-retained-fixture.service"

    def candidate(self, state=("active", "exited")):
        candidate = value()
        unit = unit_value(self.history)
        unit.update(category="retained-terminal", Transient="yes", UnitFileState="transient",
                    FragmentPath="/run/systemd/transient/" + self.history, ActiveState=state[0],
                    SubState=state[1], InvocationID="a" * 32, ExecMainStartTimestampMonotonic="10",
                    ExecMainExitTimestampMonotonic="20")
        candidate["units"][self.history] = unit
        candidate["pins"][unit["FragmentPath"]] = "sha256:" + "2" * 64
        return candidate

    def native(self, candidate):
        native = maintenance.Native(candidate, None, maintenance.Window(candidate))
        def call(args):
            if args[1] in {"list-unit-files", "list-units"}:
                # Retained transient services can occur in only list-units.
                names = candidate["units"] if args[1] == "list-units" else maintenance.transition.UNITS
                return "\n".join(name + " inactive" for name in names).encode()
            return "\n".join(k + "=" + v for k, v in candidate["units"][args[-1]].items()
                             if k != "category").encode()
        native.call = mock.Mock(side_effect=call)
        return native

    def test_retained_terminal_states_require_complete_pins_and_preserve_inventory(self):
        for state in (("active", "exited"), ("failed", "failed"), ("inactive", "dead")):
            candidate = self.candidate(state)
            self.assertEqual(maintenance.contract(files.canonical(candidate)), candidate)
            native = self.native(candidate)
            with mock.patch.object(maintenance, "empty_cgroup") as empty:
                native.units()
            self.assertIn(mock.call(self.history, native.window), empty.call_args_list)
            self.assertTrue(all(c.args[0][1] in {"show", "list-units", "list-unit-files"}
                                for c in native.call.call_args_list))
            del candidate["pins"][candidate["units"][self.history]["FragmentPath"]]
            with self.assertRaises(ERROR):
                maintenance.contract(files.canonical(candidate))

    def test_retained_live_restarting_queued_or_unbound_service_is_refused(self):
        changes = ({"ActiveState": "active", "SubState": "running"}, {"SubState": "auto-restart"},
                   {"MainPID": "1"}, {"ControlPID": "1"}, {"Job": "123"}, {"Restart": "always"},
                   {"Transient": "no"}, {"FragmentPath": "/etc/systemd/system/" + self.history},
                   {"DropInPaths": "/run/systemd/transient/extra.conf"}, {"NeedDaemonReload": "yes"},
                   {"InvocationID": ""}, {"ExecMainStartTimestampMonotonic": "21"},
                   {"ControlGroup": "/other.slice/fixture"}, {"Slice": "other.slice"})
        for change in changes:
            with self.subTest(change=change):
                candidate = self.candidate()
                candidate["units"][self.history].update(change)
                with self.assertRaises(ERROR):
                    maintenance.contract(files.canonical(candidate))

    def unstarted_failure(self):
        candidate = self.candidate(("failed", "failed"))
        candidate["unit_graph"] = "quiescent-observed"
        candidate["units"][self.history].update(
            ExecMainStartTimestampMonotonic="0", ExecMainExitTimestampMonotonic="0",
            ActiveEnterTimestampMonotonic="0", InactiveEnterTimestampMonotonic="30", NRestarts="0")
        return candidate

    def test_unstarted_failed_transient_preserves_zero_timestamps_and_native_cgroup_checks(self):
        candidate = self.unstarted_failure()
        original = copy.deepcopy(candidate)
        self.assertEqual(maintenance.contract(files.canonical(candidate)), original)
        native = self.native(candidate)
        with mock.patch.object(maintenance, "empty_cgroup") as empty:
            native.units()
        self.assertIn(mock.call(self.history, native.window), empty.call_args_list)
        self.assertEqual(candidate, original)
        self.assertTrue(all(c.args[0][1] in {"show", "list-units", "list-unit-files"}
                            for c in native.call.call_args_list))

    def test_unstarted_failure_rejects_active_partial_timestamp_restart_or_queued_history(self):
        changes = (
            {"ActiveState": "active", "SubState": "exited"},
            {"ActiveState": "inactive", "SubState": "dead"},
            {"ActiveEnterTimestampMonotonic": "1"}, {"InactiveEnterTimestampMonotonic": "0"},
            {"ExecMainStartTimestampMonotonic": "1"}, {"ExecMainExitTimestampMonotonic": "1"},
            {"NRestarts": "1"}, {"MainPID": "1"}, {"ControlPID": "1"}, {"Job": "123"},
            {"Restart": "always"}, {"Transient": "no"}, {"UnitFileState": "disabled"},
            {"InvocationID": ""}, {"DropInPaths": "/run/systemd/transient/extra.conf"},
            {"OnFailure": "other.service"}, {"Triggers": "other.service"},
            {"TriggeredBy": "other.timer"}, {"ControlGroup": "/other.slice/fixture"})
        for change in changes:
            candidate = self.unstarted_failure()
            candidate["units"][self.history].update(change)
            with self.subTest(change=change), self.assertRaises(ERROR):
                maintenance.contract(files.canonical(candidate))

    def test_unstarted_failure_requires_explicit_graph_and_complete_fragment_pin(self):
        candidate = self.unstarted_failure()
        candidate["unit_graph"] = "reciprocal"
        with self.assertRaises(ERROR):
            maintenance.contract(files.canonical(candidate))
        candidate = self.unstarted_failure()
        del candidate["pins"][candidate["units"][self.history]["FragmentPath"]]
        with self.assertRaises(ERROR):
            maintenance.contract(files.canonical(candidate))
        candidate = self.persistent_failure()
        candidate["units"][self.history].update(
            ExecMainStartTimestampMonotonic="0", ExecMainExitTimestampMonotonic="0")
        with self.assertRaises(ERROR):
            maintenance.contract(files.canonical(candidate))

    def test_unstarted_failure_cannot_bypass_native_activity_or_changed_snapshot(self):
        candidate = self.unstarted_failure()
        native = self.native(candidate)
        with mock.patch.object(maintenance, "empty_cgroup",
                               side_effect=ERROR("PREUPGRADE_TEST_POPULATED_CGROUP")), self.assertRaises(ERROR):
            native.units()
        native = self.native(candidate)
        call = native.call.side_effect
        def changed(args):
            result = call(args)
            if args[1] == "show" and args[-1] == self.history:
                return result.replace(b"ExecMainStartTimestampMonotonic=0",
                                      b"ExecMainStartTimestampMonotonic=1")
            return result
        native.call.side_effect = changed
        with mock.patch.object(maintenance, "empty_cgroup"), self.assertRaises(ERROR):
            native.units()

    def test_stopped_timer_socket_path_and_writer_guards_are_not_relaxed(self):
        for suffix in ("timer", "socket", "path", "service"):
            name = "lagrange-activation." + suffix
            candidate = self.candidate()
            candidate["units"][name] = unit_value(name)
            candidate["pins"][candidate["units"][name]["FragmentPath"]] = "sha256:" + "3" * 64
            for change in ({"ActiveState": "active"}, {"Job": "123"}, {"category": "retained-terminal"}):
                bad = copy.deepcopy(candidate)
                bad["units"][name].update(change)
                with self.subTest(name=name, change=change), self.assertRaises(ERROR):
                    maintenance.contract(files.canonical(bad))

    def test_trigger_edges_require_stopped_internal_reciprocal_units(self):
        candidate = self.candidate()
        timer = "lagrange-fixture.timer"
        service = "lagrange-fixture.service"
        for name in (timer, service):
            candidate["units"][name] = unit_value(name)
            candidate["pins"][candidate["units"][name]["FragmentPath"]] = "sha256:" + "3" * 64
        candidate["units"][timer]["Triggers"] = service
        candidate["units"][service]["TriggeredBy"] = timer
        maintenance.contract(files.canonical(candidate))
        for name, key, changed in ((timer, "Triggers", self.history), (service, "TriggeredBy", ""),
                                  (service, "TriggeredBy", "outside.timer"),
                                  (timer, "Triggers", service + " " + service),
                                  (self.history, "OnSuccess", service),
                                  (self.history, "OnFailure", service),
                                  (self.history, "OnFailureOf", "outside.service"),
                                  (self.history, "OnSuccessOf", "outside.service")):
            bad = copy.deepcopy(candidate)
            bad["units"][name][key] = changed
            with self.subTest(key=key, changed=changed), self.assertRaises(ERROR):
                maintenance.contract(files.canonical(bad))

    def test_missing_extra_and_mid_observation_inventory_changes_are_refused(self):
        for change in ("missing", "extra", "late"):
            native = self.native(self.candidate())
            original = native.call.side_effect
            counts = {"lists": 0}
            def observe(args):
                result = original(args)
                if args[1] in {"list-unit-files", "list-units"}:
                    counts["lists"] += 1
                    if change == "missing":
                        return b""
                    if change == "extra" or (change == "late" and counts["lists"] > 2):
                        return result + b"\nlagrange-unexpected.service inactive"
                return result
            native.call.side_effect = observe
            with self.subTest(change=change), mock.patch.object(maintenance, "empty_cgroup"), \
                    self.assertRaises(ERROR):
                native.units()

    def test_generation_changes_and_duplicate_properties_are_refused(self):
        for key in ("InvocationID", "ExecMainStartTimestampMonotonic", "NRestarts", "duplicate"):
            native = self.native(self.candidate())
            original = native.call.side_effect
            counts = {"history": 0}
            def observe(args):
                result = original(args)
                if args[1] == "show" and args[-1] == self.history:
                    counts["history"] += 1
                    if counts["history"] == 2:
                        if key == "duplicate":
                            return result + b"\nMainPID=0"
                        return b"\n".join((key + "=999").encode() if row.startswith((key + "=").encode())
                                          else row for row in result.splitlines())
                return result
            native.call.side_effect = observe
            with self.subTest(key=key), mock.patch.object(maintenance, "empty_cgroup"), self.assertRaises(ERROR):
                native.units()
    def persistent_failure(self):
        candidate = self.candidate(("failed", "failed"))
        candidate["unit_graph"] = "quiescent-observed"
        unit = candidate["units"][self.history]
        del candidate["pins"][unit["FragmentPath"]]
        unit.update(category="retained-disabled-failure", Transient="no", UnitFileState="disabled",
                    FragmentPath="/etc/systemd/system/" + self.history,
                    InactiveEnterTimestampMonotonic="30")
        candidate["pins"][unit["FragmentPath"]] = "sha256:" + "4" * 64
        timer = "lagrange-retained-fixture.timer"
        candidate["units"][timer] = unit_value(timer)
        candidate["units"][timer]["Triggers"] = self.history
        candidate["pins"][candidate["units"][timer]["FragmentPath"]] = "sha256:" + "5" * 64
        return candidate

    def test_disabled_persistent_failure_preserves_exact_one_way_observation_and_cgroup_gate(self):
        candidate = self.persistent_failure()
        original = copy.deepcopy(candidate)
        self.assertEqual(maintenance.contract(files.canonical(candidate)), original)
        native = self.native(candidate)
        with mock.patch.object(maintenance, "empty_cgroup") as empty:
            native.units()
        self.assertIn(mock.call(self.history, native.window), empty.call_args_list)
        self.assertEqual(candidate, original)
        self.assertEqual(candidate["units"][self.history]["TriggeredBy"], "")
        self.assertTrue(all(c.args[0][1] in {"show", "list-units", "list-unit-files"}
                            for c in native.call.call_args_list))

    def test_disabled_failure_rejects_live_enabled_hooked_or_unbound_history(self):
        for change in ({"UnitFileState": "enabled"}, {"Transient": "yes"}, {"Restart": "always"},
                       {"MainPID": "1"}, {"ControlPID": "1"}, {"Job": "10"},
                       {"ActiveState": "active", "SubState": "running"}, {"OnFailure": "x.service"},
                       {"DropInPaths": "/etc/systemd/system/extra.conf"}, {"NeedDaemonReload": "yes"},
                       {"InvocationID": ""}, {"InactiveEnterTimestampMonotonic": "19"},
                       {"ExecMainStartTimestampMonotonic": "0"}, {"category": "stopped"}):
            candidate = self.persistent_failure()
            candidate["units"][self.history].update(change)
            with self.subTest(change=change), self.assertRaises(ERROR):
                maintenance.contract(files.canonical(candidate))
        candidate = self.persistent_failure()
        del candidate["pins"][candidate["units"][self.history]["FragmentPath"]]
        with self.assertRaises(ERROR):
            maintenance.contract(files.canonical(candidate))

    def test_quiescent_graph_accepts_reported_directions_without_synthesizing_links(self):
        timer, service = "lagrange-direction.timer", "lagrange-direction.service"
        for direction in ("forward", "reverse", "both"):
            candidate = value()
            candidate["unit_graph"] = "quiescent-observed"
            for name in (timer, service):
                candidate["units"][name] = unit_value(name)
                candidate["pins"][candidate["units"][name]["FragmentPath"]] = "sha256:" + "6" * 64
            if direction in {"forward", "both"}:
                candidate["units"][timer]["Triggers"] = service
            if direction in {"reverse", "both"}:
                candidate["units"][service]["TriggeredBy"] = timer
            original = copy.deepcopy(candidate)
            self.assertEqual(maintenance.contract(files.canonical(candidate)), original)
            self.assertEqual(candidate, original)
            if direction != "both":
                candidate["unit_graph"] = "reciprocal"
                with self.assertRaises(ERROR):
                    maintenance.contract(files.canonical(candidate))

    def test_quiescent_graph_rejects_armed_external_or_enabled_failed_activation(self):
        timer = "lagrange-retained-fixture.timer"
        for name, key, changed in ((timer, "ActiveState", "active"), (timer, "SubState", "waiting"),
                                  (timer, "Job", "12"), (timer, "UnitFileState", "enabled"),
                                  (timer, "Triggers", "outside.service"),
                                  (self.history, "TriggeredBy", "outside.timer"),
                                  (self.history, "Triggers", timer)):
            candidate = self.persistent_failure()
            candidate["units"][name][key] = changed
            with self.subTest(key=key, changed=changed), self.assertRaises(ERROR):
                maintenance.contract(files.canonical(candidate))
        candidate = self.persistent_failure()
        candidate["unit_graph"] = "anything"
        with self.assertRaises(ERROR):
            maintenance.contract(files.canonical(candidate))

    def test_retained_failure_cgroup_and_reported_graph_drift_remain_hard_failures(self):
        native = self.native(self.persistent_failure())
        with mock.patch.object(maintenance, "empty_cgroup", side_effect=ERROR("PREUPGRADE_CGROUP")), \
                self.assertRaises(ERROR):
            native.units()
        original = native.call.side_effect
        def drift(args):
            data = original(args)
            if args[1] == "show" and args[-1] == self.history:
                return data.replace(b"TriggeredBy=\n", b"TriggeredBy=lagrange-retained-fixture.timer\n")
            return data
        native.call.side_effect = drift
        with mock.patch.object(maintenance, "empty_cgroup"), self.assertRaises(ERROR):
            native.units()


class SourceAncestryTests(unittest.TestCase):
    pgpath = Path("/data/docker/volumes/fixture/_data")

    def pinned(self):
        return files.Root("postgres", str(self.pgpath), 999, 999, 0o1777,
                          ancestry=tuple(tuple(row) for row in ancestry_value(self.pgpath, actual=True)))

    def isolated(self, exercise):
        """All real UID/ACL/path operations occur below a disposable chroot."""
        if os.geteuid() != 0 or sys.platform != "linux":
            self.fail("actual root Linux fixture required")
        with tempfile.TemporaryDirectory(prefix="preupgrade-source-", dir="/var/lib") as directory:
            stage = Path(directory)
            target = stage / str(self.pgpath)[1:]
            target.mkdir(parents=True, mode=0o700)
            os.chown(stage / "data", 1000, 1000)
            (stage / "data").chmod(0o755)
            target.parent.chmod(0o700)
            os.chown(target, 999, 999)
            target.chmod(0o1777)
            reader, writer = os.pipe()
            pid = os.fork()
            if pid == 0:
                os.close(reader)
                signal.alarm(10)
                try:
                    os.chroot(stage)
                    os.chdir("/")
                    exercise()
                except BaseException as error:
                    os.write(writer, (type(error).__name__ + ": " + str(error)).encode()[:2048])
                    os._exit(1)
                os._exit(0)
            os.close(writer)
            try:
                _, status = os.waitpid(pid, 0)
                error = os.read(reader, 4096).decode()
                self.assertEqual(status, 0, error)
            finally:
                os.close(reader)

    def test_explicit_source_ancestry_contract_is_complete_and_bound_to_named_volume(self):
        candidate = value()
        candidate["roots"][-1].update(path=str(self.pgpath), mode=0o1777)
        candidate["postgres"]["mountpoint"] = str(self.pgpath)
        candidate["postgres_ancestry"] = ancestry_value(self.pgpath, mode=0o1777)
        candidate["postgres_ancestry"][1][2:] = [1000, 1000, 0o755]
        candidate["postgres_ancestry"][-2][4] = 0o700
        self.assertEqual(maintenance.contract(files.canonical(candidate)), candidate)
        for changed in ([], candidate["postgres_ancestry"][:-1],
                        [*candidate["postgres_ancestry"], candidate["postgres_ancestry"][-1]]):
            bad = copy.deepcopy(candidate)
            bad["postgres_ancestry"] = changed
            with self.assertRaises((ERROR, files.SnapshotError)):
                maintenance.contract(files.canonical(bad))
        for index, field, changed in ((1, 2, 1001), (1, 3, 1001), (1, 4, 0o775),
                                      (2, 2, 1000), (0, 0, True), (-1, 1, 0)):
            bad = copy.deepcopy(candidate)
            bad["postgres_ancestry"][index][field] = changed
            with self.subTest(index=index, field=field), self.assertRaises(files.SnapshotError):
                maintenance.contract(files.canonical(bad))
        bad = copy.deepcopy(candidate)
        bad["postgres"]["volume"] = "different"
        with self.assertRaises(ERROR):
            maintenance.contract(files.canonical(bad))

    def test_actual_pinned_user_owned_data_ancestor_is_read_only(self):
        def exercise():
            root = self.pinned()
            before = ancestry_value(self.pgpath, actual=True)
            with files.open_root(root) as fd:
                self.assertEqual(files.ancestor_identity(os.fstat(fd)), root.ancestry[-1])
            self.assertEqual(ancestry_value(self.pgpath, actual=True), before)
        self.isolated(exercise)

    def test_actual_user_owned_ancestor_without_explicit_pins_is_refused(self):
        def exercise():
            root = files.Root("postgres", str(self.pgpath), 999, 999, 0o1777)
            with self.assertRaises(files.SnapshotError), files.open_root(root):
                self.fail("unpinned source opened")
        self.isolated(exercise)

    def test_actual_source_inode_mismatch_is_refused_before_yield(self):
        def exercise():
            original = self.pinned()
            for index in range(len(original.ancestry)):
                pins = [list(row) for row in original.ancestry]
                pins[index][1] += 1
                root = files.Root("postgres", str(self.pgpath), 999, 999, 0o1777,
                                  ancestry=tuple(tuple(row) for row in pins))
                with self.assertRaises(files.SnapshotError), files.open_root(root):
                    self.fail("changed inode opened")
        self.isolated(exercise)

    def test_actual_source_parent_replacement_is_refused_at_exit(self):
        def exercise():
            with self.assertRaises(files.SnapshotError), files.open_root(self.pinned()):
                os.rename("/data/docker", "/data/old-docker")
                os.mkdir("/data/docker", 0o700)
        self.isolated(exercise)

    def test_actual_source_ancestor_mode_or_owner_drift_is_refused(self):
        for mutation in (lambda: os.chmod("/data", 0o777), lambda: os.chown("/data", 1001, 1000)):
            def exercise():
                with self.assertRaises(files.SnapshotError), files.open_root(self.pinned()):
                    mutation()
            self.isolated(exercise)

    def test_actual_source_ancestor_acl_is_refused(self):
        def exercise():
            root = self.pinned()
            acl = files.struct.pack("<I", 2) + b"".join(files.struct.pack("<HHI", *entry) for entry in (
                (1, 7, 0xffffffff), (2, 5, 1001), (4, 5, 0xffffffff),
                (16, 5, 0xffffffff), (32, 5, 0xffffffff)))
            os.setxattr("/data", "system.posix_acl_access", acl)
            with self.assertRaises(files.SnapshotError), files.open_root(root):
                self.fail("ACL source opened")
        self.isolated(exercise)

    def test_actual_sticky_source_parent_stays_private_and_is_never_repaired(self):
        def exercise():
            self.pgpath.parent.chmod(0o701)
            with self.assertRaises(files.SnapshotError), files.open_root(self.pinned()):
                self.fail("shared sticky-volume parent opened")
            self.assertEqual(stat.S_IMODE(self.pgpath.parent.stat().st_mode), 0o701)
        self.isolated(exercise)

    def test_source_ancestry_cannot_be_applied_to_daily_or_other_roots(self):
        def exercise():
            root = self.pinned()
            for name in files.CLASSES:
                with self.assertRaises(files.SnapshotError):
                    files.Root(name, root.path, 10001, 10001, 0o750, ancestry=root.ancestry)
            with self.assertRaises(files.SnapshotError):
                files.Snapshot([root], files.Limits(1024**2), profile="daily")
            self.assertEqual(files.Root("postgres", "/var/lib/private", 999, 999, 0o700).ancestry, ())
        self.isolated(exercise)


class CgroupTests(unittest.TestCase):
    name = "lagrange-retained-fixture.service"

    def setUp(self):
        if os.geteuid() != 0 or sys.platform != "linux":
            self.fail("actual root Linux fixture required")
        self.temporary = tempfile.TemporaryDirectory(prefix="preupgrade-cgroup-", dir="/var/lib")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "system.slice").mkdir()
        self.window = maintenance.Window(value())

    def group(self, path):
        path.mkdir()
        for name, data in (("cgroup.procs", b""), ("cgroup.threads", b""),
                           ("cgroup.events", b"populated 0\nfrozen 0\n")):
            (path / name).write_bytes(data)
        return path

    def check(self):
        maintenance.empty_cgroup(self.name, self.window, str(self.root))

    def test_absent_and_recursively_empty_cgroups_are_read_only(self):
        self.check()
        group = self.group(self.root / "system.slice" / self.name)
        self.group(group / "child")
        before = {str(p): p.read_bytes() for p in self.root.rglob("*") if p.is_file()}
        self.check()
        self.assertEqual(before, {str(p): p.read_bytes() for p in self.root.rglob("*") if p.is_file()})

    def test_nested_process_thread_and_populated_state_are_refused(self):
        group = self.group(self.root / "system.slice" / self.name)
        child = self.group(group / "child")
        for filename, data in (("cgroup.procs", b"123\n"), ("cgroup.threads", b"456\n"),
                               ("cgroup.events", b"populated 1\nfrozen 0\n")):
            path = child / filename
            original = path.read_bytes()
            path.write_bytes(data)
            with self.subTest(filename=filename), self.assertRaises(ERROR):
                self.check()
            path.write_bytes(original)

    def test_symlink_missing_event_and_unprotected_cgroup_refuse(self):
        group = self.group(self.root / "system.slice" / self.name)
        (group / "escape").symlink_to(self.root, target_is_directory=True)
        with self.assertRaises((ERROR, OSError)):
            self.check()
        (group / "escape").unlink()
        group.chmod(0o777)
        with self.assertRaises(ERROR):
            self.check()
        group.chmod(0o755)
        (group / "cgroup.events").unlink()
        with self.assertRaises(OSError):
            self.check()

    def test_cgroup_depth_and_deadline_are_bounded(self):
        group = self.group(self.root / "system.slice" / self.name)
        for _ in range(17):
            group = self.group(group / "child")
        with self.assertRaises(ERROR):
            self.check()
        with mock.patch.object(self.window, "remaining", side_effect=ERROR("PREUPGRADE_EXPIRED")), \
                self.assertRaises(ERROR):
            self.check()

    def test_cgroup_generation_replacement_during_read_refuses(self):
        group = self.group(self.root / "system.slice" / self.name)
        original = maintenance.os.read
        replaced = []
        def read(fd, limit):
            data = original(fd, limit)
            if not replaced:
                replaced.append(True)
                group.rename(group.with_name("old"))
                self.group(group)
            return data
        with mock.patch.object(maintenance.os, "read", side_effect=read), self.assertRaises(ERROR):
            self.check()


class WriterPolicyTests(unittest.TestCase):
    """Pure metadata regression: no native commands, secrets or service changes."""

    def container(self):
        value = fixture.PreupgradeRecoveryTests.container()
        value["HostConfig"] = {"Binds": ["first", "second"], "ReadonlyRootfs": True}
        value["NetworkSettings"] = {"Networks": {"fixture": {"NetworkID": "f" * 64}}}
        value["Mounts"] = [
            {"Type": "bind", "Source": "/fixture/" + name, "Destination": "/data/" + name,
             "Mode": "ro", "RW": False, "Propagation": "rprivate"}
            for name in ("curated", "catalog", "artifacts", "key")
        ]
        return value

    def test_writer_policy_all_mount_permutations_match_without_mutating_input(self):
        value = self.container()
        original = copy.deepcopy(value)
        expected = maintenance.stopped_writer(value)
        for permutation in itertools.permutations(value["Mounts"]):
            observed = copy.deepcopy(value)
            observed["Mounts"] = list(permutation)
            self.assertEqual(maintenance.stopped_writer(observed), expected)
        self.assertEqual(value, original)

    def test_writer_policy_mount_field_changes_remain_visible(self):
        value = self.container()
        expected = maintenance.stopped_writer(value)
        for key, replacement in (("Source", "/changed"), ("Destination", "/other"),
                                 ("RW", True), ("Mode", "rw"), ("Propagation", "shared"),
                                 ("Type", "volume"), ("Name", "added-field")):
            with self.subTest(field=key):
                observed = copy.deepcopy(value)
                observed["Mounts"][0][key] = replacement
                self.assertNotEqual(maintenance.stopped_writer(observed), expected)

    def test_writer_policy_other_arrays_and_network_fields_keep_their_meaning(self):
        value = self.container()
        expected = maintenance.stopped_writer(value)
        for change in (
            lambda v: v["HostConfig"]["Binds"].reverse(),
            lambda v: v["Config"].update(Env=["changed"]),
            lambda v: v["NetworkSettings"]["Networks"]["fixture"].update(NetworkID="e" * 64),
        ):
            observed = copy.deepcopy(value)
            change(observed)
            self.assertNotEqual(maintenance.stopped_writer(observed), expected)

    def test_writer_policy_rejects_ambiguous_or_malformed_mounts(self):
        value = self.container()
        for mounts in (None, {}, [value["Mounts"][0]] * 2, [{}],
                       [{"Destination": "/data/../curated"}],
                       [{"Destination": "/data//curated"}],
                       [{"Destination": "/data/\ncurated"}],
                       [{"Destination": "/data/" + str(i)} for i in range(65)]):
            with self.subTest(mounts=mounts):
                observed = copy.deepcopy(value)
                observed["Mounts"] = mounts
                with self.assertRaises(ERROR):
                    maintenance.stopped_writer(observed)

    def test_writer_policy_still_requires_a_stopped_generation(self):
        value = self.container()
        for patch in ({"Running": True}, {"Pid": 1}, {"Paused": True}, {"OOMKilled": True}):
            observed = copy.deepcopy(value)
            observed["State"].update(patch)
            with self.assertRaises(ERROR):
                maintenance.stopped_writer(observed)


if __name__ == "__main__":
    unittest.main(verbosity=2)
