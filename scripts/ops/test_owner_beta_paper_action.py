#!/usr/bin/env python3
"""Focused caller/action binding and v2 custody filesystem checks."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import pathlib
import sys
import uuid
import unittest
from unittest import mock

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import test_owner_beta_paper_journal as core  # noqa: E402

journal = core.journal
FIXTURE = HERE.parents[1] / "tests/fixtures/owner-beta-paper-operations/action-bindings-v1.json"
EXPECTED_FIXTURE_SHA256 = "ee980b0a63ca52ba26c06eafa0d4b970e9843d85e5fcddb271c10e786186c7ea"


def H(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def object_bytes(vector, name):
    return vector["objects"][name]["canonical"].encode("ascii")


class SharedActionVectorTests(unittest.TestCase):
    def test_shared_originals_and_bound_negative_vectors(self):
        raw = FIXTURE.read_bytes()
        self.assertEqual(hashlib.sha256(raw).hexdigest(), EXPECTED_FIXTURE_SHA256)
        vector = json.loads(raw)
        self.assertEqual(len(vector["objects"]), 96)
        self.assertEqual(len(vector["traces"]), 6)
        self.assertEqual(len(vector["negative_cases"]), 44)
        values = {}
        for name, item in vector["objects"].items():
            content = item["canonical"].encode("ascii")
            self.assertEqual(H(content), item["sha256"])
            values[name] = journal._json(content, journal.MAX_NATIVE_INDEX)
        traces = {trace["name"]: trace for trace in vector["traces"]}
        validated_dispatches = 0
        for trace in vector["traces"]:
            root = values[trace["root"]]
            journal._validate_intent(object_bytes(vector, trace["root"]))
            caller = journal._validate_caller(object_bytes(vector, trace["caller"]), root["installation_id"])
            self.assertEqual(caller["generation"], root["generation"])
            self.assertEqual(caller["root_intent_sha256"], H(object_bytes(vector, trace["root"])))
            self.assertEqual(caller["command_canonical"].encode("ascii"), journal._command_projection(root))
            previous = None
            for index, action_entry in enumerate(trace["actions"]):
                action = journal._validate_action(object_bytes(vector, action_entry["binding"]), root,
                                                  index, previous)
                previous = H(object_bytes(vector, action_entry["binding"]))
                self.assertEqual(action["sequence"], index)
                evidence_hash = None if action_entry["evidence"] is None else vector["objects"][action_entry["evidence"]]["sha256"]
                self.assertEqual(action["artifact_sha256"], evidence_hash)
                if action["stage_sha256"] is not None:
                    self.assertIn(action["stage_sha256"], {vector["objects"][name]["sha256"] for name in trace["stages"]})
                evidence_bytes = None if action_entry["evidence"] is None else object_bytes(vector, action_entry["evidence"])
                dispatch_bytes = object_bytes(vector, action_entry["dispatch"])
                dispatch, _originals = journal._validate_v2_dispatch(
                    dispatch_bytes, object_bytes(vector, trace["root"]),
                    object_bytes(vector, action_entry["binding"]), evidence_bytes)
                self.assertEqual(H(dispatch["sql_request_canonical"].encode("ascii")),
                                 vector["objects"][action_entry["sql"]]["sha256"])
                journal._validate_bound_request(dispatch["sql_request_canonical"].encode("ascii"),
                                                root, action, evidence_bytes)
                validated_dispatches += 1
        self.assertEqual(validated_dispatches, 13)
        relevant = 0
        for negative in vector["negative_cases"]:
            name = negative.get("object")
            if name not in vector["objects"]:
                continue
            item = vector["objects"][name]
            definition = item["definition"]
            if definition not in {"intent", "callerBinding", "actionBinding"}:
                continue
            value = json.loads(item["canonical"])
            if negative["mode"] != "replace":
                continue
            value[negative["field"]] = negative["replacement"]
            changed = core.canonical(value)
            with self.subTest(negative=negative["name"]), self.assertRaises(journal.JournalError):
                if definition == "intent":
                    mutated_intent = journal._validate_intent(changed)
                    trace = traces[name.split(".")[0]]
                    root_hash = H(changed)
                    caller = values[trace["caller"]]
                    if caller["root_intent_sha256"] != root_hash:
                        raise journal.JournalError("JOURNAL_IDENTITY")
                    if any(values[entry["binding"]]["root_intent_sha256"] != root_hash
                           for entry in trace["actions"]):
                        raise journal.JournalError("JOURNAL_IDENTITY")
                elif definition == "callerBinding":
                    caller = journal._validate_caller(changed)
                    trace = traces[name.split(".")[0]]
                    root = values[trace["root"]]
                    if caller["generation"] != root["generation"] or caller["root_intent_sha256"] != vector["objects"][trace["root"]]["sha256"]:
                        raise journal.JournalError("JOURNAL_IDENTITY")
                    if caller["command_canonical"].encode("ascii") != journal._command_projection(root):
                        raise journal.JournalError("JOURNAL_IDENTITY")
                else:
                    trace = traces[name.split(".")[0]]
                    root = values[trace["root"]]
                    entry = next(entry for entry in trace["actions"] if entry["binding"] == name)
                    index = trace["actions"].index(entry)
                    prior = None if index == 0 else H(object_bytes(vector, trace["actions"][index - 1]["binding"]))
                    action = journal._validate_action(changed, root, index, prior)
                    evidence_hash = None if entry["evidence"] is None else vector["objects"][entry["evidence"]]["sha256"]
                    if action["artifact_sha256"] != evidence_hash:
                        raise journal.JournalError("JOURNAL_IDENTITY")
                    if action["stage_sha256"] is not None:
                        stages = {vector["objects"][stage]["sha256"] for stage in trace["stages"]}
                        if action["stage_sha256"] not in stages:
                            raise journal.JournalError("JOURNAL_IDENTITY")
            relevant += 1
        self.assertGreaterEqual(relevant, 16)


class ActionFilesystemTests(core.JournalFixture):
    def _action(self, operation, sequence, *, verb="begin", key=None, artifact=None, stage=None,
                expected_epoch=None, previous=None):
        return core.canonical({
            "schema": "owner-beta-paper-action-binding-v1",
            "installation_id": self.installation_id,
            "generation": operation.generation,
            "root_intent_sha256": operation.intent_sha256,
            "sequence": sequence,
            "previous_binding_sha256": previous,
            "verb": verb,
            "request_key": key or operation.generation + ":" + verb,
            "expected_epoch": 0 if verb == "configure" else (3 if expected_epoch is None else expected_epoch),
            "artifact_sha256": None if artifact is None else H(artifact),
            "stage_sha256": stage,
        })

    def test_caller_replay_conflict_and_original_v2_dispatch_readback(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            root = self.intent()
            projection = journal._command_projection(json.loads(root))
            generation = json.loads(root)["generation"]
            operation = store.bind_caller("caller-command-7", root)
            self.assertEqual(operation.generation, generation)
            before = sorted(os.listdir(store._callers))
            operation.close()
            replay_root = json.loads(root)
            replay_root["generation"] = str(uuid.uuid4())
            replay_root["request_key"] = replay_root["generation"] + ":begin"
            replay = store.bind_caller("caller-command-7", core.canonical(replay_root))
            self.assertEqual(replay.generation, generation)
            self.assertEqual(sorted(os.listdir(store._callers)), before)
            replay.close()
            changed = json.loads(root)
            changed["release_manifest_sha256"] = "sha256:" + "b" * 64
            with self.assertRaises(journal.JournalError):
                store.bind_caller("caller-command-7", core.canonical(changed))

            operation = store.open(generation)
            stage0 = self.stage(operation, "INTENT", 0, None)
            stage_hash = operation.append(stage0)
            action_bytes = self._action(operation, 0, stage=stage_hash)
            action = operation.prepare_action(action_bytes)
            duplicate = operation.prepare_action(action_bytes)
            self.assertEqual(duplicate.sequence, 0)
            duplicate.close()
            changed_action = json.loads(action_bytes)
            changed_action["expected_epoch"] = 4
            with self.assertRaises(journal.JournalError):
                operation.prepare_action(core.canonical(changed_action))

            native_input = self.base / "native-input"
            native_input.mkdir(mode=0o700)
            native_fd = os.open(native_input, os.O_RDONLY | os.O_DIRECTORY)
            try:
                action.stage_inputs(native_fd)
                self.assertEqual((native_input / "request.json").read_bytes(), root)
                self.assertEqual((native_input / "action-binding.json").read_bytes(), action_bytes)
            finally:
                os.close(native_fd)

            sql = core.canonical({"schema": "owner-beta-paper-exact-request-v1", "kind": "BEGIN",
                                  "account_id": json.loads(root)["account_id"],
                                  "args": {"expected_epoch": 3, "intent_canonical": root.decode("ascii"),
                                           "request_key": generation + ":begin"}})
            dispatch = core.canonical({
                "schema": "owner-beta-paper-native-dispatch-v2",
                "root_intent_canonical": root.decode("ascii"), "root_intent_sha256": H(root),
                "action_canonical": action_bytes.decode("ascii"), "action_sha256": H(action_bytes),
                "artifact_canonical": None, "artifact_sha256": None,
                "sql_request_canonical": sql.decode("ascii"), "sql_request_sha256": H(sql),
            })
            for content in (root, action_bytes, sql):
                journal._new_file(action._output_fd, H(content)[7:] + ".json", content, 0o400)
            journal._new_file(action._output_fd, H(dispatch)[7:] + ".json", dispatch, 0o400)
            journal._new_file(action._output_fd, "dispatch.json", dispatch, 0o400)
            self.assertEqual(os.fstat(action.borrow_output_fd()).st_ino, os.fstat(action._output_fd).st_ino)
            custody = action.import_dispatch()
            readback = self.base / "readback"
            readback.mkdir(mode=0o700)
            readback_fd = os.open(readback, os.O_RDONLY | os.O_DIRECTORY)
            try:
                custody.stage_readback(readback_fd)
                self.assertEqual((readback / "request.json").read_bytes(), root)
                self.assertEqual((readback / "action-binding.json").read_bytes(), action_bytes)
                self.assertEqual((readback / "sql-request.json").read_bytes(), sql)
            finally:
                os.close(readback_fd)
            custody.close()
            action.close()
            operation.close()

            # Readback is independent of the live stage chain. It still requires all R/A/E/Q originals.
            op_path = self.install / journal.PAPER_DIR / "operations" / generation
            (op_path / "00000000.json").unlink()
            read_only = store.open_custody(generation, 0)
            self.assertFalse(hasattr(read_only, "append"))
            self.assertEqual(read_only._sql, sql)
            readback2 = self.base / "readback-direct"
            readback2.mkdir(mode=0o700)
            readback2_fd = os.open(readback2, os.O_RDONLY | os.O_DIRECTORY)
            try:
                read_only.stage_readback(readback2_fd)
                self.assertEqual((readback2 / "request.json").read_bytes(), root)
                self.assertEqual((readback2 / "action-binding.json").read_bytes(), action_bytes)
                self.assertEqual((readback2 / "sql-request.json").read_bytes(), sql)
            finally:
                os.close(readback2_fd)
            read_only.close()

    def test_readonly_custody_rejects_named_original_replacement(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            root = self.intent()
            operation = store.create(root)
            stage_hash = operation.append(self.stage(operation, "INTENT", 0, None))
            action_bytes = self._action(operation, 0, stage=stage_hash)
            action = operation.prepare_action(action_bytes)
            sql = core.canonical({"schema": "owner-beta-paper-exact-request-v1", "kind": "BEGIN",
                                  "account_id": json.loads(root)["account_id"],
                                  "args": {"expected_epoch": 3, "intent_canonical": root.decode("ascii"),
                                           "request_key": operation.generation + ":begin"}})
            dispatch = core.canonical({"schema": "owner-beta-paper-native-dispatch-v2",
                                       "root_intent_canonical": root.decode("ascii"),
                                       "root_intent_sha256": H(root),
                                       "action_canonical": action_bytes.decode("ascii"),
                                       "action_sha256": H(action_bytes), "artifact_canonical": None,
                                       "artifact_sha256": None, "sql_request_canonical": sql.decode("ascii"),
                                       "sql_request_sha256": H(sql)})
            for content in (root, action_bytes, sql):
                journal._new_file(action._output_fd, H(content)[7:] + ".json", content, 0o400)
            journal._new_file(action._output_fd, H(dispatch)[7:] + ".json", dispatch, 0o400)
            journal._new_file(action._output_fd, "dispatch.json", dispatch, 0o400)
            imported = action.import_dispatch()
            self.assertEqual(os.fstat(imported.borrow_output_fd()).st_ino, os.fstat(action._output_fd).st_ino)
            imported.close()
            action.close()
            operation.close()
            custody = store.open_custody(json.loads(root)["generation"], 0)
            source = (self.install / journal.PAPER_DIR / "operations" / json.loads(root)["generation"] /
                      "actions" / "00000000" / "output" / (H(sql)[7:] + ".json"))
            replacement = self.base / "sql-copy.json"
            replacement.write_bytes(sql)
            os.chmod(replacement, 0o400)
            source.unlink()
            replacement.rename(source)
            destination = self.base / "readback-replaced"
            destination.mkdir(mode=0o700)
            destination_fd = os.open(destination, os.O_RDONLY | os.O_DIRECTORY)
            try:
                with self.assertRaises(journal.JournalError):
                    custody.stage_readback(destination_fd)
            finally:
                os.close(destination_fd)
                custody.close()

    def test_action_publication_fsync_failure_remains_incomplete(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            operation = store.create(self.intent())
            stage_hash = operation.append(self.stage(operation, "INTENT", 0, None))
            action_bytes = self._action(operation, 0, stage=stage_hash)
            real_fsync = os.fsync
            calls = 0

            def fail_action_directory_fsync(fd):
                nonlocal calls
                calls += 1
                if calls == 2:
                    raise OSError("test fsync boundary")
                return real_fsync(fd)

            with mock.patch.object(journal.os, "fsync", fail_action_directory_fsync):
                with self.assertRaises(OSError):
                    operation.prepare_action(action_bytes)
            with self.assertRaises(journal.JournalError):
                operation.revalidate()
            operation.close()

    def test_action_sequence_forks_and_missing_or_torn_index_fail(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            intent = self.intent()
            operation = store.create(intent)
            stage_hash = operation.append(self.stage(operation, "INTENT", 0, None))
            skipped = self._action(operation, 1, verb="reconcile", key=operation.generation + ":reconcile:1",
                                   artifact=core.canonical({"sample": 1}), stage=stage_hash, expected_epoch=4)
            with self.assertRaises(journal.JournalError):
                operation.prepare_action(skipped, core.canonical({"sample": 1}))
            action_bytes = self._action(operation, 0, stage=stage_hash)
            action = operation.prepare_action(action_bytes)
            root = operation._intent_bytes
            sql = core.canonical({"schema": "owner-beta-paper-exact-request-v1", "kind": "BEGIN",
                                  "account_id": json.loads(root)["account_id"],
                                  "args": {"expected_epoch": 3, "intent_canonical": root.decode("ascii"),
                                           "request_key": operation.generation + ":begin"}})
            dispatch = core.canonical({"schema": "owner-beta-paper-native-dispatch-v2",
                                       "root_intent_canonical": root.decode("ascii"),
                                       "root_intent_sha256": H(root),
                                       "action_canonical": action_bytes.decode("ascii"),
                                       "action_sha256": H(action_bytes), "artifact_canonical": None,
                                       "artifact_sha256": None, "sql_request_canonical": sql.decode("ascii"),
                                       "sql_request_sha256": H(sql)})
            for content in (root, action_bytes, sql):
                journal._new_file(action._output_fd, H(content)[7:] + ".json", content, 0o400)
            journal._new_file(action._output_fd, "dispatch.json", dispatch, 0o400)
            with self.assertRaises(journal.JournalError):
                action.import_dispatch()  # The content-addressed index must already exist.
            journal._new_file(action._output_fd, H(dispatch)[7:] + ".json", dispatch, 0o400)
            q_path = pathlib.Path(self.install) / journal.PAPER_DIR / "operations" / operation.generation / "actions" / "00000000" / "output" / (H(sql)[7:] + ".json")
            q_path.unlink()
            with self.assertRaises(journal.JournalError):
                action.import_dispatch()  # Missing Q cannot be reconstructed from dispatch.json.
            journal._new_file(action._output_fd, H(sql)[7:] + ".json", sql, 0o400)
            (pathlib.Path(self.install) / journal.PAPER_DIR / "operations" / operation.generation /
             "actions" / "00000000" / "import.json").write_bytes(b"{}")
            os.chmod(pathlib.Path(self.install) / journal.PAPER_DIR / "operations" / operation.generation /
                     "actions" / "00000000" / "import.json", 0o400)
            with self.assertRaises(journal.JournalError):
                action.revalidate()
            action.close()
            operation.close()

    def test_scope_and_restore_destination_stage_contract(self):
        with self.opened() as store:
            self.installation_id = store.installation_id
            bad_backup = json.loads(self.intent(purpose="BACKUP"))
            bad_backup["scope"] = "PAPER_STOP"
            with self.assertRaises(journal.JournalError):
                journal._validate_intent(core.canonical(bad_backup))
            bad_stop = json.loads(self.intent())
            bad_stop["scope"] = "COMBINED_BACKUP"
            with self.assertRaises(journal.JournalError):
                journal._validate_intent(core.canonical(bad_stop))
            journal._advance("RECONCILED", "RESTORE_VERIFIED", {"purpose": "RESTORE"})
            journal._advance("RESTORE_VERIFIED", "RESTORED_COMMITTED", {"purpose": "RESTORE"})
            journal._advance("OFF_COMMITTED", "SNAPSHOT", {"purpose": "BACKUP"})
            for prior, current in (("RECONCILED", "OFF_COMMITTED"), ("OFF_COMMITTED", "SNAPSHOT")):
                with self.assertRaises(journal.JournalError):
                    journal._advance(prior, current, {"purpose": "RESTORE"})
            with self.assertRaises(journal.JournalError):
                journal._advance("OFF_COMMITTED", "FAILED", {"purpose": "STOP"})

    def test_action_purpose_swaps_and_reconcile_ordinal_are_rejected(self):
        vector = json.loads(FIXTURE.read_bytes())
        traces = {trace["name"]: trace for trace in vector["traces"]}
        cases = (("restore", 2, "off", ":off"),
                 ("stop", 3, "restore-off", ":restore-off"),
                 ("stop", 1, "reconcile", ":reconcile:2"))
        for trace_name, sequence, verb, suffix in cases:
            trace = traces[trace_name]
            root = journal._validate_intent(object_bytes(vector, trace["root"]))
            entry = trace["actions"][sequence]
            value = json.loads(object_bytes(vector, entry["binding"]))
            value["verb"] = verb
            value["request_key"] = root["generation"] + suffix
            previous = None if sequence == 0 else H(object_bytes(vector, trace["actions"][sequence - 1]["binding"]))
            with self.subTest(trace=trace_name, sequence=sequence, verb=verb), self.assertRaises(journal.JournalError):
                journal._validate_action(core.canonical(value), root, sequence, previous)


if __name__ == "__main__":
    unittest.main()
