#!/usr/bin/env python3
"""Synthetic evidence and archive regressions; no filesystem-policy/DB execution."""
import contextlib
import hashlib
import importlib.util
import io
import json
import struct
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]


def load(name, relative):
    spec = importlib.util.spec_from_file_location(name, ROOT / relative)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


ops = load("simulation_backup_operations_test", "scripts/ops/lib/owner-beta-simulation-operations.py")
files = load("simulation_backup_files_test", "scripts/ops/lib/owner-beta-daily-backup-files.py")
fixtures = load("simulation_backup_release_fixture", "scripts/ops/test_owner_beta_simulation_release.py")
daily_backup = load("simulation_daily_backup_test", "scripts/ops/lib/owner-beta-daily-backup.py")
backup_tool = load("simulation_backup_tool_test", "scripts/ops/lib/production-backup-tool.py")

SYNTHETIC_CANDIDATE = "1" * 64
SYNTHETIC_BARS = b"synthetic historical bars\n"
SYNTHETIC_UNSIGNED = {"candidate_content_sha256": "sha256:" + SYNTHETIC_CANDIDATE, "schema_version": 3}
SYNTHETIC_UNSIGNED_BYTES = json.dumps(SYNTHETIC_UNSIGNED, separators=(",", ":")).encode()
SYNTHETIC_MANIFEST_HASH = "sha256:" + hashlib.sha256(SYNTHETIC_UNSIGNED_BYTES).hexdigest()
SYNTHETIC_MANIFEST = json.dumps({**SYNTHETIC_UNSIGNED, "manifest_sha256": SYNTHETIC_MANIFEST_HASH},
                                separators=(",", ":")).encode() + b"\n"
SYNTHETIC_APPROVAL = {"candidate_content_sha256": "sha256:" + SYNTHETIC_CANDIDATE,
    "artifact_manifest_sha256": SYNTHETIC_MANIFEST_HASH,
    "bars_sha256": "sha256:" + hashlib.sha256(SYNTHETIC_BARS).hexdigest(),
    "bars_size_bytes": len(SYNTHETIC_BARS)}
SYNTHETIC_REGISTRY = json.dumps({"schema_id": "kis-historical-price-only-v3-approval-registry",
    "schema_version": 1, "approved_artifacts": [SYNTHETIC_APPROVAL]}, separators=(",", ":")).encode()


def evidence_fixture():
    data = fixtures.binding_fixture()
    binding = ops.runtime.Binding("synthetic", "/synthetic/releases/" + fixtures.COMMIT,
                                  fixtures.COMMIT, data["config_data"])
    return binding, {
        "simulation_config/config.json": data["config_data"],
        "simulation_receipt/artifact.json": data["receipt_data"],
        "simulation_receipt/COMPLETE": data["complete"],
        "simulation_manifest/.lagrange-release-manifest": data["manifest"],
        "simulation_floor/.owner-beta-simulation-reader-floor": ops.release.FLOOR_BYTES,
        "simulation_reader/owner-beta-simulation-reader-v1.json": ops.release.READER_BYTES,
        "simulation_approval/" + ops.APPROVAL_NAME: SYNTHETIC_REGISTRY,
    }


def archive(profile, evidence, *, metadata_override=None, extra_input=None):
    classes, magic, schema = files.profile_values(profile)
    content = dict(evidence)
    prefix = "simulation_input/" + SYNTHETIC_CANDIDATE
    content[prefix + "/bars.ndjson"] = SYNTHETIC_BARS
    content[prefix + "/manifest.json"] = SYNTHETIC_MANIFEST
    if extra_input:
        content.update(extra_input)
    metadata = {
        "simulation_config/config.json": (0, 10001, 0o440),
        "simulation_receipt/artifact.json": (0, 0, 0o444),
        "simulation_receipt/COMPLETE": (0, 0, 0o444),
        "simulation_manifest/.lagrange-release-manifest": (0, 0, 0o600),
        "simulation_floor/.owner-beta-simulation-reader-floor": (0, 0, 0o600),
        "simulation_reader/owner-beta-simulation-reader-v1.json": (0, 0, 0o644),
        "simulation_approval/" + ops.APPROVAL_NAME: (0, 0, 0o644),
        prefix + "/bars.ndjson": (10001, 10001, 0o600),
        prefix + "/manifest.json": (10001, 10001, 0o600),
    }
    if metadata_override:
        metadata.update(metadata_override)
    entries, payloads = [], []
    for name in classes:
        uid, gid, mode = ((0, 10001, 0o750) if name == "simulation_config" else
                          (10001, 10001, 0o750) if name == "simulation_input" else (0, 0, 0o755))
        entries.append({"path": name, "kind": "directory", "uid": uid, "gid": gid,
                        "mode": mode, "size": 0, "sha256": None})
        if name == "simulation_input":
            entries.append({"path": prefix, "kind": "directory", "uid": 10001, "gid": 10001,
                            "mode": 0o700, "size": 0, "sha256": None})
        for path in sorted(p for p in content if p.startswith(name + "/")):
            data = content[path]
            uid, gid, mode = metadata.get(path, (0, 0, 0o444))
            entries.append({"path": path, "kind": "file", "uid": uid, "gid": gid,
                            "mode": mode, "size": len(data),
                            "sha256": "sha256:" + hashlib.sha256(data).hexdigest()})
            payloads.append(data)
    header = files.canonical({"schema": schema, "classes": list(classes), "entries": entries})
    return magic + struct.pack(">Q", len(header)) + header + b"".join(payloads)


class BackupEvidenceTests(unittest.TestCase):
    def setUp(self):
        pin = "sha256:" + hashlib.sha256(SYNTHETIC_REGISTRY).hexdigest()
        patch_pin = mock.patch.object(ops, "APPROVAL_REGISTRY_SHA256", pin)
        patch_read = mock.patch.object(ops, "read_approval", return_value=(SYNTHETIC_REGISTRY, SYNTHETIC_APPROVAL))
        patch_pin.start()
        patch_read.start()
        self.addCleanup(patch_pin.stop)
        self.addCleanup(patch_read.stop)

    def test_valid_simulation_only_archive_binds_receipt_floor_and_release(self):
        binding, evidence = evidence_fixture()
        data = archive("simulation", evidence)
        result = ops.verify_simulation_archive(io.BytesIO(data), files.Limits(1000000), binding,
                                               profile="simulation")
        self.assertEqual(result["classes"], list(files.SIMULATION_CLASSES))
        self.assertNotIn("daily", result["classes"])

    def test_combined_archive_keeps_daily_and_exact_approved_simulation_tree(self):
        binding, evidence = evidence_fixture()
        data = archive("daily-simulation", evidence)
        result = ops.verify_simulation_archive(io.BytesIO(data), files.Limits(1000000), binding,
                                               profile="daily-simulation")
        self.assertEqual(result["classes"], list(files.DAILY_SIMULATION_CLASSES))
        self.assertIn("daily", result["classes"])
        self.assertIn("simulation_approval", result["classes"])

    def test_malformed_stale_or_partial_evidence_is_rejected(self):
        binding, evidence = evidence_fixture()
        for key in evidence:
            with self.subTest(key=key):
                changed = dict(evidence)
                changed[key] += b"stale"
                with self.assertRaises(Exception):
                    ops.verify_archived_evidence(changed, binding)
                changed = dict(evidence)
                del changed[key]
                with self.assertRaises(Exception):
                    ops.verify_archived_evidence(changed, binding)

    def test_bad_modes_and_truncated_archive_are_rejected(self):
        binding, evidence = evidence_fixture()
        bad = archive("simulation", evidence, metadata_override={"simulation_receipt/COMPLETE": (0, 0, 0o600)})
        with self.assertRaises(ops.OperationError):
            ops.verify_simulation_archive(io.BytesIO(bad), files.Limits(1000000), binding, profile="simulation")
        good = archive("simulation", evidence)
        with self.assertRaises(Exception):
            ops.verify_simulation_archive(io.BytesIO(good[:-1]), files.Limits(1000000), binding,
                                          profile="simulation")

    def test_unrelated_input_sibling_is_not_admitted_into_archive(self):
        binding, evidence = evidence_fixture()
        data = archive("simulation", evidence, extra_input={"simulation_input/unrelated": b"not approved"})
        with self.assertRaises(ops.OperationError):
            ops.verify_simulation_archive(io.BytesIO(data), files.Limits(1000000), binding,
                                          profile="simulation")

    def test_backup_state_rejects_partial_stale_and_wrong_pins(self):
        binding, _ = evidence_fixture()
        config = ops.release.validate_config(binding.config_data, binding.commit)
        state = {"schema": "lagrange-simulation-backup-state-v1", "profile": "simulation",
                 "application_commit": binding.commit, "tool_commit": "b2" * 20,
                 "tool_manifest_sha256": "sha256:" + "b" * 64,
                 "manifest_sha256": config["manifest_sha256"], "config_sha256": binding.config_sha256,
                 "receipt_sha256": binding.receipt_sha256, "database_sha256": "sha256:" + "c" * 64,
                 "daily_database": None, "files_sha256": "sha256:" + "d" * 64}
        self.assertFalse(daily_backup.validate_simulation_state(state, binding, binding.commit))
        for key, replacement in (("application_commit", "c3" * 20), ("config_sha256", "sha256:" + "f" * 64),
                                 ("receipt_sha256", "sha256:" + "e" * 64), ("tool_manifest_sha256", True),
                                 ("files_sha256", "bad")):
            with self.subTest(key=key):
                changed = dict(state)
                changed[key] = replacement
                with self.assertRaises(daily_backup.runtime.RuntimeErrorCode):
                    daily_backup.validate_simulation_state(changed, binding, binding.commit)
        changed = dict(state)
        del changed["receipt_sha256"]
        with self.assertRaises(daily_backup.runtime.RuntimeErrorCode):
            daily_backup.validate_simulation_state(changed, binding, binding.commit)

    def test_daily_format_remains_distinct_and_cannot_be_relabelled(self):
        binding, evidence = evidence_fixture()
        self.assertEqual(files.profile_values("daily"),
                         (files.CLASSES, files.MAGIC, "lagrange-daily-files-v1"))
        data = archive("simulation", evidence)
        with self.assertRaises(Exception):
            ops.verify_simulation_archive(io.BytesIO(data), files.Limits(1000000), binding,
                                          profile="daily-simulation")

    def test_actual_cli_dispatch_selects_simulation_or_unchanged_daily(self):
        release = "/synthetic/install/releases/" + fixtures.COMMIT
        values = {"LAGRANGE_CODE_COMMIT": fixtures.COMMIT}
        backup = {"LOCK_FILE": "/synthetic/backup.lock"}
        database = object()
        for required in (True, False):
            with self.subTest(simulation=required), contextlib.ExitStack() as stack:
                stack.enter_context(mock.patch.object(daily_backup, "ROOT", release))
                stack.enter_context(mock.patch.object(daily_backup.install, "require_root"))
                stack.enter_context(mock.patch.object(daily_backup.os, "umask"))
                stack.enter_context(mock.patch.object(daily_backup.signal, "signal", return_value=None))
                stack.enter_context(mock.patch.object(daily_backup.ops.policy, "validate_current_release"))
                stack.enter_context(mock.patch.object(daily_backup.ops, "read_manifest", return_value={"db-migrate": "fixture"}))
                stack.enter_context(mock.patch.object(daily_backup.ops, "project_binding", return_value=("synthetic", "absent")))
                stack.enter_context(mock.patch.object(daily_backup.ops, "read_context", return_value=(values, [])))
                stack.enter_context(mock.patch.object(daily_backup.ops, "reject_ambient"))
                stack.enter_context(mock.patch.object(daily_backup, "config_file", return_value=backup))
                stack.enter_context(mock.patch.object(daily_backup.ops, "release_lock", return_value=contextlib.nullcontext()))
                stack.enter_context(mock.patch.object(daily_backup, "backup_lock", return_value=contextlib.nullcontext()))
                stack.enter_context(mock.patch.object(daily_backup.runtime, "Docker", return_value=object()))
                stack.enter_context(mock.patch.object(daily_backup.simulation.runtime, "Docker", return_value=object()))
                stack.enter_context(mock.patch.object(daily_backup.simulation, "inspect_transition",
                    return_value=(SimpleNamespace(required=required), database)))
                sim_run = stack.enter_context(mock.patch.object(daily_backup, "simulation_backup_run", return_value="SIMULATION_VERIFIED_DISABLED"))
                stack.enter_context(mock.patch.object(daily_backup.ops, "activation_binding",
                    return_value=(object(), {"raw_owner_uid": 10001})))
                stack.enter_context(mock.patch.object(daily_backup, "roots"))
                stack.enter_context(mock.patch.object(daily_backup.ops, "Operations", return_value=object()))
                stack.enter_context(mock.patch.object(daily_backup, "Storage"))
                controller = stack.enter_context(mock.patch.object(daily_backup, "Controller"))
                controller.return_value.backup.return_value = "VERIFIED_DISABLED"
                output = stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
                result = daily_backup.main(["--operation", "backup", "--mode", "apply", "--install-root", "/synthetic/install"])
                self.assertEqual(result, 0)
                if required:
                    self.assertEqual(sim_run.call_args.kwargs["action"], "backup")
                    controller.return_value.backup.assert_not_called()
                    self.assertIn("SIMULATION_VERIFIED_DISABLED", output.getvalue())
                else:
                    sim_run.assert_not_called()
                    controller.return_value.backup.assert_called_once()

    def test_capture_selects_simulation_only_or_combined_and_never_resumes(self):
        binding, _ = evidence_fixture()
        context = SimpleNamespace(release=binding.release, commit=binding.commit, images={}, values={})
        operation = SimpleNamespace(required=True, binding=binding, source_release=binding.release,
            target_release=binding.release, source_commit=binding.commit, target_commit=binding.commit,
            install_root="/synthetic/install", project="synthetic", source_images={"api-server": "fixture"},
            values={"LAGRANGE_RUNTIME_SECRET_DIR": "/synthetic/secrets"})
        backup = {"DATA_ROOT": "/synthetic/data"}
        for combined in (False, True):
            with self.subTest(combined=combined), contextlib.ExitStack() as stack:
                storage = mock.Mock()
                storage.limits = files.Limits(1000000)
                storage.capture_simulation.return_value = contextlib.nullcontext(("source", "final", "name"))
                storage.prune.return_value = (1, 1)
                stack.enter_context(mock.patch.object(daily_backup, "SimulationStorage", return_value=storage))
                stack.enter_context(mock.patch.object(daily_backup.simulation.runtime, "capture",
                    return_value=SimpleNamespace(running=False)))
                stack.enter_context(mock.patch.object(daily_backup.simulation, "prepare_transition",
                    return_value=(object(), SimpleNamespace(generation=None))))
                stack.enter_context(mock.patch.object(daily_backup.simulation.release, "reader_guard"))
                daily_binding = object() if combined else None
                daily_config = object() if combined else None
                stack.enter_context(mock.patch.object(daily_backup, "simulation_daily_binding", return_value=(daily_binding, daily_config)))
                stack.enter_context(mock.patch.object(daily_backup.ops, "project_binding", return_value=("synthetic", "absent")))
                daily_operation = stack.enter_context(mock.patch.object(daily_backup.ops, "Operations"))
                stack.enter_context(mock.patch.object(daily_backup.ops, "global_cohort"))
                stack.enter_context(mock.patch.object(daily_backup.runtime, "capture", return_value={}))
                stack.enter_context(mock.patch.object(daily_backup, "roots", return_value=()))
                stack.enter_context(mock.patch.object(daily_backup, "publication_fence", return_value=contextlib.nullcontext()))
                stack.enter_context(mock.patch.object(daily_backup, "raw_publication_fence", return_value=contextlib.nullcontext()))
                snapshot = mock.Mock()
                snapshot.manifest.return_value = b"{}"
                make_snapshot = stack.enter_context(mock.patch.object(daily_backup, "simulation_snapshot",
                    return_value=contextlib.nullcontext(snapshot)))
                stack.enter_context(mock.patch.object(daily_backup.simulation, "simulation_audit", return_value="sha256:" + "c" * 64))
                stack.enter_context(mock.patch.object(daily_backup, "bound_database"))
                stack.enter_context(mock.patch.object(daily_backup.files, "database_references"))
                stack.enter_context(mock.patch.object(daily_backup.simulation, "verify_drained"))
                stack.enter_context(mock.patch.object(daily_backup.simulation, "verify_fence"))
                database = mock.Mock()
                database.backup_audit.return_value = {"fixture": True}
                result = daily_backup.simulation_backup_run(operation, context, database, object(), object(), backup,
                    tool_commit=binding.commit, tool_hash=None, verifier_root=None, action="backup")
                self.assertEqual(result, "SIMULATION_VERIFIED_DISABLED")
                self.assertEqual(storage.capture_simulation.call_args.args[3],
                                 "daily-simulation" if combined else "simulation")
                self.assertEqual(make_snapshot.call_args.kwargs["daily_roots"], () if combined else None)
                storage.publish.assert_called_once_with("source", "final", "name", simulation_set=True)
                if combined:
                    daily_operation.return_value.disable.assert_called_once()
                    daily_operation.return_value.enable.assert_not_called()

    def test_maintenance_commit_and_application_commit_are_independent(self):
        app = "a1" * 20
        tool_commit = "b2" * 20
        tool_root = Path("/synthetic/install/maintenance") / tool_commit
        payloads = {name: ("synthetic:" + name).encode() for name in backup_tool.REQUIRED}
        members = {name: {"sha256": backup_tool.digest(data), "bytes": len(data), "mode": 0o644}
                   for name, data in payloads.items()}
        manifest = files.canonical({"schema": backup_tool.SCHEMA, "tool_commit": tool_commit,
                                    "application_commit": app, "members": members})
        payloads[backup_tool.MANIFEST] = manifest
        def read(path, **_kwargs):
            return payloads[str(Path(path).relative_to(tool_root))]
        with mock.patch.object(backup_tool.files, "read_root_file", side_effect=read):
            self.assertEqual(backup_tool.read_manifest(tool_root, app, install_root=Path("/synthetic/install"),
                                                       expected_hash=backup_tool.digest(manifest)),
                             backup_tool.digest(manifest))
            with self.assertRaises(ValueError):
                backup_tool.read_manifest(tool_root, "c3" * 20, install_root=Path("/synthetic/install"))
            with self.assertRaises(ValueError):
                backup_tool.read_manifest(tool_root, app, install_root=Path("/synthetic/install"),
                                          expected_hash="sha256:" + "0" * 64)


LEGACY_CANDIDATE = "2" * 64
LEGACY_BARS = b"synthetic preserved legacy bars\n"
LEGACY_UNSIGNED = {"candidate_content_sha256": "sha256:" + LEGACY_CANDIDATE,
                   "bars": {"sha256": "sha256:" + hashlib.sha256(LEGACY_BARS).hexdigest(),
                            "size_bytes": len(LEGACY_BARS)}, "schema_version": 2}
LEGACY_HASH = "sha256:" + hashlib.sha256(json.dumps(LEGACY_UNSIGNED, separators=(",", ":")).encode()).hexdigest()
LEGACY_MANIFEST = json.dumps({**LEGACY_UNSIGNED, "manifest_sha256": LEGACY_HASH},
                             separators=(",", ":")).encode() + b"\n"
LEGACY_RECORD = {"candidate_content_sha256": "sha256:" + LEGACY_CANDIDATE,
                 "artifact_manifest_sha256": LEGACY_HASH}
LEGACY_REGISTRY = json.dumps({"schema_id": "kis-historical-price-only-beta-approval-registry",
    "schema_version": 2, "approved_artifacts": [LEGACY_RECORD]}, separators=(",", ":")).encode()


def preserved_legacy_archive(*, unknown=False, changed_bars=False, missing_legacy=False, diverge_alias=False):
    binding, evidence = evidence_fixture()
    original = archive("daily-simulation", evidence)
    magic = files.DAILY_SIMULATION_MAGIC
    size = struct.unpack(">Q", original[len(magic):len(magic) + 8])[0]
    manifest = json.loads(original[len(magic) + 8:len(magic) + 8 + size])
    offset = len(magic) + 8 + size
    content = {}
    for entry in manifest["entries"]:
        if entry["kind"] == "file":
            content[entry["path"]] = original[offset:offset + entry["size"]]
            offset += entry["size"]
    legacy = "simulation_input/kis-historical-price-only-beta"
    for suffix in ("", "/v2", "/v2/candidate-sha256=" + LEGACY_CANDIDATE):
        manifest["entries"].append(dict(path=legacy + suffix, kind="directory", uid=10001,
                                       gid=10001, mode=0o700, size=0, sha256=None))
    prefix = legacy + "/v2/candidate-sha256=" + LEGACY_CANDIDATE
    for name, data in [("manifest.json", LEGACY_MANIFEST), ("bars.ndjson", LEGACY_BARS)]:
        if missing_legacy and name == "manifest.json":
            continue
        if changed_bars and name == "bars.ndjson":
            data += b"changed"
        content[prefix + "/" + name] = data
        manifest["entries"].append(dict(path=prefix + "/" + name, kind="file", uid=10001,
            gid=10001, mode=0o600, size=len(data), sha256="sha256:" + hashlib.sha256(data).hexdigest()))
    if unknown:
        path = "simulation_input/unapproved"
        content[path] = b"unapproved sibling"
        manifest["entries"].append(dict(path=path, kind="file", uid=10001, gid=10001, mode=0o600,
            size=len(content[path]), sha256="sha256:" + hashlib.sha256(content[path]).hexdigest()))
    # Combined capture retains exactly the same complete source tree in both aliases.
    for entry in list(manifest["entries"]):
        if entry["path"].startswith("simulation_input/"):
            target = "base/" + entry["path"].removeprefix("simulation_input/")
            copied = {**entry, "path": target}
            if entry["kind"] == "file":
                content[target] = content[entry["path"]]
                if diverge_alias and entry["path"] == prefix + "/bars.ndjson":
                    content[target] += b"different alias"
                    copied.update(size=len(content[target]),
                                  sha256="sha256:" + hashlib.sha256(content[target]).hexdigest())
            manifest["entries"].append(copied)
    manifest["entries"].sort(key=lambda entry: (manifest["classes"].index(entry["path"].split("/")[0]),
                                              tuple(entry["path"].split("/")[1:])))
    header = files.canonical(manifest)
    payload = b"".join(content[e["path"]] for e in manifest["entries"] if e["kind"] == "file")
    return binding, magic + struct.pack(">Q", len(header)) + header + payload


class PreservedLegacyHistoryTests(unittest.TestCase):
    def setUp(self):
        BackupEvidenceTests.setUp(self)
        patches = (
            mock.patch.object(daily_backup, "simulation", ops),
            mock.patch.object(daily_backup, "LEGACY_APPROVAL_SHA256",
                              "sha256:" + hashlib.sha256(LEGACY_REGISTRY).hexdigest()),
            mock.patch.object(daily_backup, "read_legacy_approval",
                              side_effect=lambda _: daily_backup.legacy_approved_record(LEGACY_REGISTRY)),
        )
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)

    def test_inactive_capture_retains_existing_approved_legacy_with_v3(self):
        binding, data = preserved_legacy_archive()
        storage = object.__new__(daily_backup.InactiveDailySimulationStorage)
        storage.limits, storage.simulation_binding = files.Limits(1000000), binding
        result = storage.verify_archive(io.BytesIO(data), profile="daily-simulation")
        paths = {entry["path"] for entry in result["entries"]}
        self.assertIn("simulation_input/kis-historical-price-only-beta/v2/candidate-sha256=" +
                      LEGACY_CANDIDATE + "/bars.ndjson", paths)
        self.assertIn("base/" + SYNTHETIC_CANDIDATE + "/bars.ndjson", paths)

    def test_normal_simulation_verifier_still_rejects_legacy_siblings(self):
        binding, data = preserved_legacy_archive()
        with self.assertRaises(ops.OperationError):
            ops.verify_simulation_archive(io.BytesIO(data), files.Limits(1000000), binding,
                                          profile="daily-simulation")

    def test_unknown_sibling_is_rejected_despite_valid_legacy_approval(self):
        binding, data = preserved_legacy_archive(unknown=True)
        with self.assertRaisesRegex(Exception, "INACTIVE_BACKUP_LEGACY_TREE"):
            daily_backup.verify_inactive_simulation_archive(io.BytesIO(data), files.Limits(1000000),
                                                            binding, profile="daily-simulation")

    def test_changed_legacy_bars_with_valid_archive_digests_are_rejected(self):
        binding, data = preserved_legacy_archive(changed_bars=True)
        with self.assertRaisesRegex(Exception, "INACTIVE_BACKUP_INPUT_BYTES"):
            daily_backup.verify_inactive_simulation_archive(io.BytesIO(data), files.Limits(1000000),
                                                            binding, profile="daily-simulation")

    def test_complete_input_aliases_cannot_disagree_or_omit_legacy_bytes(self):
        binding, data = preserved_legacy_archive(diverge_alias=True)
        with self.assertRaisesRegex(Exception, "INACTIVE_BACKUP_INPUT_ALIAS"):
            daily_backup.verify_inactive_simulation_archive(io.BytesIO(data), files.Limits(1000000),
                                                            binding, profile="daily-simulation")

    def test_missing_legacy_manifest_and_changed_registry_fail_closed(self):
        binding, data = preserved_legacy_archive(missing_legacy=True)
        with self.assertRaises(Exception):
            daily_backup.verify_inactive_simulation_archive(io.BytesIO(data), files.Limits(1000000),
                                                            binding, profile="daily-simulation")
        with self.assertRaisesRegex(Exception, "INACTIVE_BACKUP_LEGACY_APPROVAL"):
            daily_backup.legacy_approved_record(LEGACY_REGISTRY + b" ")

    def test_minimal_v3_archive_retains_its_existing_validation(self):
        binding, evidence = evidence_fixture()
        data = archive("daily-simulation", evidence)
        self.assertEqual(daily_backup.verify_inactive_simulation_archive(
            io.BytesIO(data), files.Limits(1000000), binding, profile="daily-simulation"),
            ops.verify_simulation_archive(io.BytesIO(data), files.Limits(1000000), binding,
                                           profile="daily-simulation"))


if __name__ == "__main__":
    unittest.main()
