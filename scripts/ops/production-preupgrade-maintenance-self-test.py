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
import os
from pathlib import Path
import sys
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
    units = {name: {"LoadState": "loaded", "ActiveState": "inactive", "SubState": "dead",
        "FragmentPath": "/etc/systemd/system/" + name, "DropInPaths": "", "NeedDaemonReload": "no",
        "UnitFileState": "disabled", "ActiveEnterTimestampMonotonic": "0",
        "InactiveEnterTimestampMonotonic": "1", "ExecMainStartTimestampMonotonic": "0"}
        for name in maintenance.transition.UNITS}
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
            "networks": {"lagrange_backend": "f" * 64}}, "roots": roots, "writers": [], "units": units,
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
        missing = value()
        del missing["pins"]["/opt/lagrange/maintenance/" + missing["tool_commit"] + "/" + maintenance.PAYLOAD[0]]
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
        for change in (lambda c: c["Mounts"].append({"Source": self.value["roots"][0]["path"]}),
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
            unit = dict(candidate["units"][args[-1]])
            unit["ExecMainStartTimestampMonotonic"] = "999"
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


if __name__ == "__main__":
    unittest.main(verbosity=2)
