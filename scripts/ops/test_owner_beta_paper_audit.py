import copy
import hashlib
import importlib.util
import io
import json
import sys
import types
from decimal import Decimal
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("paper_audit", Path(__file__).parent / "lib/owner-beta-paper-audit.py")
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)
DOC = json.loads(audit.PROJECTION.read_bytes())["archive_audit_v1"]


def stream(rows=None):
    rows = rows or {}
    records = [["header", "owner-beta-paper-db-stream-v1", audit.digest(audit.PROJECTION.read_bytes())]]
    for group, tables in sorted(DOC["paper_source_table_projection"].items()):
        for table in tables:
            records.extend(["row", group, table, key, row] for key, row in rows.get(table, []))
            records.append(["table_end", group, table])
        records.append(["group_end", group])
    for table in DOC["source_reference_tables"]:
        records.extend(["reference", table, key, row] for key, row in rows.get(table, []))
    return records + [["references_end"], ["end"]]


def wire(records):
    return b"".join(audit.canonical(r) + b"\n" for r in records)


def account(name="가😀"):
    row = {c: None for c in DOC["archive_table_columns"]["accounts"]}
    row.update(id="11111111-1111-1111-1111-111111111111", owner_user_id="22222222-2222-2222-2222-222222222222",
               account_type="PAPER", name=name, currency="KRW", status="ACTIVE", initial_cash="1.0000",
               paper_state_version=0, created_at="2026-11-06T00:00:00.000001Z", updated_at="2026-11-06T00:00:00.000001Z")
    return row


class AuditTests(unittest.TestCase):
    def decode(self, records):
        return audit.decode(io.BytesIO(wire(records)))

    def test_all_nine_explicit_empty_groups_are_descriptive(self):
        result = self.decode(stream())
        self.assertEqual(len(result["groups"]), 9)
        self.assertEqual([r["name"] for r in result["groups"]], sorted(set(DOC["groups_in_order"]) - {"source_files"}))
        self.assertTrue(all(r["sha256"] == "sha256:" + hashlib.sha256(b"").hexdigest() for r in result["groups"]))
        self.assertNotEqual(result["schema"], "owner-beta-paper-audit-v1")
        self.assertEqual(result["source_files"], "REQUIRED_EXTERNAL_EVIDENCE")

    def test_independent_unicode_frame_hash(self):
        row = account()
        result = self.decode(stream({"accounts": [(row["id"], row)]}))
        # Independent JSON serialization and byte framing (no production frame/canonical).
        encoded = json.dumps(row, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()
        parts = [b"accounts", row["id"].encode(), encoded]
        expected = b"".join(str(len(v)).encode() + b":" + v for v in parts)
        group = next(g for g in result["groups"] if g["name"] == "paper_control")
        self.assertEqual(group["sha256"], "sha256:" + hashlib.sha256(expected).hexdigest())
        self.assertEqual(group["byte_length"], len(expected))

    def test_length_prefix_ambiguity(self):
        self.assertNotEqual(audit.frame("ab", "c", {}), audit.frame("a", "bc", {}))
        self.assertNotEqual(audit.frame("x", "12:a", "b"), audit.frame("x", "1", "2:ab"))

    def test_numeric_and_string_semantics(self):
        self.assertEqual(audit.canonical(audit.parse(b'{"z":1e2,"a":1.000,"x":-0.0}')), b'{"a":1,"x":0,"z":100}')
        variants = [True, None, Decimal(1), "1", False, "null"]
        self.assertEqual(len(set(audit.canonical(v) for v in variants)), len(variants))
        self.assertEqual(audit.canonical({"é": "😀", "a": "\n"}), '{"a":"\\n","é":"😀"}'.encode())

    def test_mutation_and_deletion_change_root(self):
        row = account()
        before = self.decode(stream({"accounts": [(row["id"], row)]}))["groups"]
        row["initial_cash"] = "2.0000"
        after = self.decode(stream({"accounts": [(row["id"], row)]}))["groups"]
        self.assertNotEqual(before, after)
        self.assertNotEqual(after, self.decode(stream())["groups"])

    def test_every_truncation_rejected(self):
        records = stream()
        for n in range(len(records)):
            with self.subTest(n=n), self.assertRaises(audit.AuditError):
                self.decode(records[:n])
        with self.assertRaises(audit.AuditError):
            audit.decode(io.BytesIO(wire(records)[:-1]))

    def test_missing_extra_reordered_groups_tables(self):
        records = stream()
        for i in (1, 2, len(records) - 4):
            bad = copy.deepcopy(records)
            bad.pop(i)
            with self.assertRaises(audit.AuditError):
                self.decode(bad)
        with self.assertRaises(audit.AuditError):
            self.decode(records + [["end"]])
        records[1], records[2] = records[2], records[1]
        with self.assertRaises(audit.AuditError):
            self.decode(records)

    def test_duplicate_and_nonordered_row_keys(self):
        row = account()
        for ids in ([row["id"], row["id"]], ["bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb", row["id"]]):
            rows = [(i, {**row, "id": i}) for i in ids]
            with self.assertRaises(audit.AuditError):
                self.decode(stream({"accounts": rows}))

    def test_malformed_duplicate_keys_and_oversize_depth(self):
        for raw in (b'{"x":1,"x":2}', b'{"x":NaN}', b'1e999999999999999999999999', b'\xef\xbb\xbf{}', b'"\xff"', b'[' * 65 + b']' * 65):
            with self.subTest(raw=raw[:10]), self.assertRaises(audit.AuditError):
                audit.parse(raw)
        with self.assertRaises(audit.AuditError):
            audit.canonical(Decimal("1e999999999"))

    def test_overflow_and_float_rejected(self):
        root = audit.Root()
        root.size = audit.I64
        with self.assertRaises(audit.AuditError):
            root.add(b"x")
        with self.assertRaises(audit.AuditError):
            audit.canonical(0.1)
        row = account()
        row["paper_state_version"] = 2 ** 63
        with self.assertRaises(audit.AuditError):
            self.decode(stream({"accounts": [(row["id"], row)]}))

    def test_receipt_exact_bytes_and_null_history(self):
        row = {k: None for k in ("request_canonical", "request_sha256", "result_canonical", "result_sha256")}
        audit._receipt(row)
        for prefix in ("request", "result"):
            row[prefix + "_canonical"] = '{"a":1}'
            row[prefix + "_sha256"] = audit.digest(b'{"a":1}')
        audit._receipt(row)
        for value in ('{"a":2}', '{ "a":1}', None):
            bad = {**row, "result_canonical": value}
            with self.assertRaises(audit.AuditError):
                audit._receipt(bad)

    def test_raw_error_prose_rejected_without_disclosure(self):
        row = {c: None for c in DOC["archive_table_columns"]["job_attempts"]}
        row.update(id=account()["id"], error_message="DO_NOT_DISCLOSE_PROVIDER_BODY")
        with self.assertRaises(audit.AuditError) as err:
            self.decode(stream({"job_attempts": [(row["id"], row)]}))
        self.assertNotIn(row["error_message"], str(err.exception))
        row["error_message"] = audit.digest(b"DO_NOT_DISCLOSE_PROVIDER_BODY")
        self.decode(stream({"job_attempts": [(row["id"], row)]}))

    def test_reference_exact_set_and_callback(self):
        row = {c: None for c in DOC["archive_table_columns"]["users"]}
        # users is R4 identity history, not itself a file reference. A Daily row is.
        row = {c: None for c in DOC["archive_table_columns"]["owner_beta_daily_poll_state"]}
        row["stage"] = "ADMISSION"
        records = stream({"owner_beta_daily_poll_state": [("ADMISSION", row)]})
        calls = []
        audit.decode(io.BytesIO(wire(records)), lambda *args: calls.append(args))
        self.assertEqual(len(calls), 1)
        records = [r for r in records if r[0] != "reference"]
        with self.assertRaises(audit.AuditError):
            self.decode(records)

    def test_fixed_scale_and_key_substitution_rejected(self):
        row = account()
        for value in ("1", "1.0", Decimal(1), True):
            bad = {**row, "initial_cash": value}
            with self.assertRaises(audit.AuditError):
                self.decode(stream({"accounts": [(row["id"], bad)]}))
        with self.assertRaises(audit.AuditError):
            self.decode(stream({"accounts": [("replaced", row)]}))

    def test_diagnostic_json_is_only_a_digest(self):
        row = {c: None for c in DOC["archive_table_columns"]["paper_rebalance_previews"]}
        row["id"] = account()["id"]
        row["error_json"] = {"code": "FAILED", "message": "DO_NOT_DISCLOSE"}
        with self.assertRaises(audit.AuditError):
            self.decode(stream({"paper_rebalance_previews": [(row["id"], row)]}))
        row["error_json"] = {"sha256": audit.digest(b'{"code":"FAILED","message":"DO_NOT_DISCLOSE"}')}
        result = self.decode(stream({"paper_rebalance_previews": [(row["id"], row)]}))
        self.assertNotIn("DO_NOT_DISCLOSE", str(result))

    def test_oversize_and_source_files_forgery(self):
        old = audit.MAX_LINE
        try:
            audit.MAX_LINE = 8
            with self.assertRaises(audit.AuditError):
                audit.decode(io.BytesIO(b'["too long"]\n'))
        finally:
            audit.MAX_LINE = old
        records = stream()
        records.insert(-2, ["group_end", "source_files"])
        with self.assertRaises(audit.AuditError):
            self.decode(records)

    def test_all_declared_types_handled(self):
        known = {"int2", "int4", "int8", "bool", "uuid", "date", "timestamptz", "bytea", "text", "varchar", "numeric", "jsonb"}
        self.assertLessEqual({v for cols in DOC["archive_column_types"].values() for v in cols.values()}, known)




class RestoreDeltaTests(unittest.TestCase):
    def fixture(self):
        ids = [f"{n:08d}-1111-1111-1111-111111111111" for n in range(1, 10)]
        account_id, generation, installation, source_generation, source_installation, fence_id, off_id, observation_id, owner = ids
        stamp = "2026-11-11T00:00:00.000001Z"
        sha = "sha256:" + "a" * 64
        def row(table, **values):
            result = {key: None for key in DOC["archive_table_columns"][table]}
            result.update(values)
            return result
        def exact(kind, args, data, epoch):
            request = audit.canonical({"schema":"owner-beta-paper-exact-request-v1", "kind":kind,
                "account_id":account_id, "args":args}).decode()
            result = audit.canonical({"schema":"owner-beta-paper-action-v1", "outcome":"OK",
                "action":{"BEGIN":"BEGIN_OPERATION","RECONCILE":"RECONCILE_OPERATION","TRANSITION":"TRANSITION"}[kind],
                "account_id":account_id, "control_epoch":epoch, "data":data}).decode()
            return dict(request_canonical=request, request_sha256=audit.digest(request.encode()),
                result_canonical=result, result_sha256=audit.digest(result.encode()))
        before = row("owner_beta_paper_control", id="etf11", account_id=account_id, owner_user_id=owner,
                     state="OFF", control_epoch=2, updated_at=stamp)
        after = {**before, "state":"RESTORED_OFF", "control_epoch":11}
        intent = {"generation":generation, "installation_id":installation, "account_id":account_id,
            "predecessor_generation":source_generation, "artifact_sha256":sha}
        operation = row("owner_beta_paper_operations", generation=generation, installation_id=installation,
            account_id=account_id, request_key=generation+":begin", purpose="RESTORE", scope="COMBINED_BACKUP", prior_state="OFF",
            expected_epoch=2, local_epoch_floor=9, fence_epoch=10, intent=intent, fence_event_id=fence_id,
            intent_sha256=audit.digest(audit.canonical(intent)),
            **exact("BEGIN", {"request_key":generation+":begin","expected_epoch":2,
                "intent_canonical":audit.canonical(intent).decode()}, {"generation":generation,
                "operation_event_id":fence_id,"intent_sha256":audit.digest(audit.canonical(intent)),"fenced_epoch":10}, 10))
        fence = row("owner_beta_paper_events", id=fence_id, owner_user_id=owner, account_id=account_id,
            control_epoch=10, kind="DRAIN_STARTED", payload=intent, occurred_at=stamp, request_key=generation+":begin",
            payload_sha256=audit.digest(audit.canonical(intent)))
        summary = {"account_id":account_id, "installation_id":installation, "generation":generation, "control_epoch":10, "inventory_sha256":sha}
        observation = row("owner_beta_paper_operation_observations", observation_id=observation_id,
            generation=generation, request_key=generation+":reconcile:1", summary=summary, observed_at=stamp,
            summary_sha256=audit.digest(audit.canonical(summary)),
            **exact("RECONCILE", {"request_key":generation+":reconcile:1","generation":generation,
                "expected_epoch":10,"inventory_sha256":sha}, {"observation_id":observation_id,
                "summary":summary,"summary_sha256":audit.digest(audit.canonical(summary))}, 10))
        evidence = {"generation":generation, "installation_id":installation, "account_id":account_id,
            "source_generation":source_generation, "source_installation_id":source_installation,
            "source_epoch":1, "destination_epoch_floor":9, "archive_sha256":sha,
            "source_audit_sha256":sha, "restored_audit_sha256":sha, "expected_epoch":2,
            "resulting_epoch":10, "observation_id":observation_id}
        off = row("owner_beta_paper_events", id=off_id, owner_user_id=owner, account_id=account_id,
            control_epoch=11, kind="RESTORED_OFF", payload=evidence, occurred_at=stamp, request_key=generation+":restore-off",
            payload_sha256=audit.digest(audit.canonical(evidence)),
            **exact("TRANSITION", {"request_key":generation+":restore-off","expected_epoch":10,
                "transition":"RESTORED_OFF","evidence_canonical":audit.canonical(evidence).decode()},
                {"event_id":off_id,"state":"RESTORED_OFF"}, 11))
        receipts = {}
        for name, item, kind, identity, epoch in (("begin",operation,"BEGIN",fence_id,10),
            ("reconcile",observation,"RECONCILE",observation_id,10), ("restore-off",off,"TRANSITION",off_id,11)):
            receipts[name] = {"schema":"owner-beta-paper-exact-readback-v1", "kind":kind,
                "request_sha256":item["request_sha256"], "status":"COMMITTED", "account_id":account_id,
                "current_epoch":epoch, "current_state":"RESTORED_OFF" if epoch==11 else "DRAINING",
                "event_id":identity, "event_epoch":epoch, "result_canonical":item["result_canonical"],
                "result_sha256":item["result_sha256"], "reset":None}
        rows = {"owner_beta_paper_control":{"etf11":{"before":before,"after":after}},
            "owner_beta_paper_operations":{generation:{"before":None,"after":operation}},
            "owner_beta_paper_operation_observations":{observation_id:{"before":None,"after":observation}},
            "owner_beta_paper_events":{fence_id:{"before":None,"after":fence},off_id:{"before":None,"after":off}}}
        expected = {"generation":generation,"installation_id":installation,"account_id":account_id,
            "source_generation":source_generation,"source_installation_id":source_installation,
            "source_epoch":1,"source_control_epoch":2,"expected_epoch":2,"destination_epoch_floor":9,
            "fence_epoch":10,"restore_epoch":11,"archive_sha256":sha,"source_audit_sha256":sha,
            "receipts":receipts,"rows":rows}
        source = {"owner_beta_paper_control":[("etf11",before)]}
        destination = {table:sorted((key,change["after"]) for key,change in changes.items()) for table,changes in rows.items()}
        return source, destination, expected

    def compare(self, original, destination, expected, **datasets):
        return audit.compare_restore_delta(io.BytesIO(wire(stream(original))), io.BytesIO(wire(stream(destination))),
            expected, source_dataset_rows=datasets.get("source", []), after_dataset_rows=datasets.get("after", []))

    def test_exact_restore_trace_accepts_fence10_off11_source_receipt_epoch1(self):
        source, destination, expected = self.fixture()
        self.assertIsNone(self.compare(source, destination, expected))

    def replace_restore_evidence(self, destination, expected, evidence):
        off = next(value for _, value in destination["owner_beta_paper_events"] if value["kind"] == "RESTORED_OFF")
        off["payload"] = evidence
        off["payload_sha256"] = audit.digest(audit.canonical(evidence))
        request = json.loads(off["request_canonical"])
        request["args"]["evidence_canonical"] = audit.canonical(evidence).decode()
        off["request_canonical"] = audit.canonical(request).decode()
        off["request_sha256"] = audit.digest(off["request_canonical"].encode())
        expected["receipts"]["restore-off"]["request_sha256"] = off["request_sha256"]

    def test_transition_epochs_cannot_replace_receipt_source_and_fence(self):
        source, destination, expected = self.fixture()
        off = next(value for _, value in destination["owner_beta_paper_events"] if value["kind"] == "RESTORED_OFF")
        self.replace_restore_evidence(destination, expected, {**off["payload"], "expected_epoch":10, "resulting_epoch":11})
        with self.assertRaisesRegex(audit.AuditError, "RESTORE_EVIDENCE"):
            self.compare(source, destination, expected)

    def test_destination_caller_receipt_preserves_sql_epoch_mapping(self):
        from unittest import mock
        # Exercise the actual receipt producer with mocked resource observations;
        # this does not stand in for native/SQL lifecycle acceptance.
        path = Path(__file__).parent / "lib/owner-beta-paper-backup.py"
        spec = importlib.util.spec_from_file_location("paper_audit_receipt_producer", path)
        backup = importlib.util.module_from_spec(spec)
        with mock.patch.dict(sys.modules, {spec.name:backup}):
            spec.loader.exec_module(backup)
        source, destination, expected = self.fixture()
        operation = next(iter(expected["rows"]["owner_beta_paper_operations"].values()))["after"]
        observation = next(iter(expected["rows"]["owner_beta_paper_operation_observations"].values()))["after"]
        off = next(value for _, value in destination["owner_beta_paper_events"] if value["kind"] == "RESTORED_OFF")
        owner = object.__new__(backup._DestinationRestore)
        owner.revalidate = mock.Mock()
        owner.package = types.SimpleNamespace(guest=types.SimpleNamespace(observe_fence=lambda _: {"database":{}}))
        owner.ops = types.SimpleNamespace(_canonical=audit.canonical, _digest=audit.digest,
            _current_boot_id=lambda: expected["installation_id"], _stage_observation=lambda: {"seconds":1,"nanoseconds":0})
        owner.operation = types.SimpleNamespace(_intent={**operation["intent"],"expected_epoch":2,"local_epoch_floor":9,
            "scope":"COMBINED_BACKUP", "release_manifest_sha256":expected["archive_sha256"]},
            intent_sha256=operation["intent_sha256"])
        owner.archive = dict(installation_id=expected["source_installation_id"],generation=expected["source_generation"],
            control_epoch=2,audit_sha256=expected["source_audit_sha256"])
        owner.archive_sha256 = expected["archive_sha256"]
        owner.fence, owner.begin_event = 10, operation["fence_event_id"]
        owner.head = (0, expected["archive_sha256"])
        owner.stage = mock.Mock()
        owner.retain = lambda value: audit.digest(audit.canonical(value))
        owner.store = types.SimpleNamespace(advance_floor=mock.Mock())
        captured = []
        def invoke(verb, evidence):
            if verb == "reconcile":
                return {"data":{"summary":{**observation["summary"],"observation_id":observation["observation_id"]}}}
            self.assertEqual(verb,"restore-off")
            captured.append(evidence)
            return {"data":{"event_id":off["id"]}}
        owner.invoke = invoke
        owner.stop_and_restore_off()
        self.assertEqual(len(captured),1)
        self.assertEqual((captured[0]["expected_epoch"],captured[0]["resulting_epoch"]),(2,10))
        self.replace_restore_evidence(destination,expected,captured[0])
        self.assertIsNone(self.compare(source,destination,expected))

    def test_receipt_identity_epoch_and_status_mutants_refuse(self):
        for field, value in (("source_epoch",2), ("fence_epoch",9), ("restore_epoch",12),
                             ("source_installation_id","bad"), ("archive_sha256","bad")):
            source, destination, expected = self.fixture()
            expected[field] = value
            with self.subTest(field=field), self.assertRaises(audit.AuditError):
                self.compare(source, destination, expected)
        for name in ("begin","reconcile","restore-off"):
            source, destination, expected = self.fixture()
            expected["receipts"][name]["status"] = "UNPROVEN"
            with self.subTest(receipt=name), self.assertRaises(audit.AuditError):
                self.compare(source, destination, expected)

    def test_expected_is_not_a_field_mask_or_permission(self):
        source, destination, expected = self.fixture()
        destination["owner_beta_paper_control"][0][1]["last_signal_date"] = "2026-11-10"
        with self.assertRaisesRegex(audit.AuditError,"RESTORE_FIELDS"):
            self.compare(source,destination,expected)
        source, destination, expected = self.fixture()
        destination["accounts"] = [(account()["id"],account())]
        with self.assertRaisesRegex(audit.AuditError,"RESTORE_UNEXPECTED_DELTA"):
            self.compare(source,destination,expected)
        expected["rows"]["accounts"] = {account()["id"]:{"before":None,"after":account()}}
        with self.assertRaisesRegex(audit.AuditError,"RESTORE_TABLES"):
            self.compare(source,destination,expected)

    def test_missing_exact_insert_and_truncated_stream_refuse(self):
        source, destination, expected = self.fixture()
        destination.pop("owner_beta_paper_operation_observations")
        with self.assertRaises(audit.AuditError):
            self.compare(source,destination,expected)
        with self.assertRaises(audit.AuditError):
            audit.compare_restore_delta(io.BytesIO(wire(stream(source))[:-1]), io.BytesIO(wire(stream(destination))),
                expected,source_dataset_rows=[],after_dataset_rows=[])

    def test_dataset_all_six_fields_and_extra_rows_refuse(self):
        source, destination, expected = self.fixture()
        row = {"id":account()["id"],"dataset_id":"x","version":"v1","status":"READY",
               "manifest_sha256":"a"*64,"storage_path":"/data/curated"}
        for field in row:
            mutant = {**row,field: "b"*64 if field=="manifest_sha256" else "mutated"}
            with self.subTest(field=field),self.assertRaises(audit.AuditError):
                self.compare(source,destination,expected,source=[row],after=[mutant])
        with self.assertRaisesRegex(audit.AuditError,"RESTORE_DATASET_SET"):
            self.compare(source,destination,expected,source=[row],after=[row])

    def test_no_economic_job_session_outbox_or_delivery_exclusions(self):
        for table in ("orders", "fills", "positions", "cash_ledger", "daily_equity", "paper_settlement_outbox",
                      "paper_settlement_outbox_archive", "notification_deliveries", "notifications", "jobs",
                      "job_attempts", "owner_beta_paper_sessions", "owner_beta_paper_session_events"):
            source, destination, expected = self.fixture()
            row = {key: None for key in DOC["archive_table_columns"][table]}
            parts = []
            for key in DOC["archive_identity_key_components"][table]:
                typ = DOC["archive_column_types"][table][key]
                value = (1 if typ.startswith("int") else "2026-11-10" if typ == "date"
                         else account()["id"] if typ == "uuid" else "fixture")
                row[key] = value
                parts.append(str(value).zfill(20) if key == "event_seq" else str(value))
            identity = ":".join(parts)
            source[table] = [(identity,row)]
            destination[table] = [(identity,copy.deepcopy(row))]
            with self.subTest(table=table):
                self.assertIsNone(self.compare(source,destination,expected))
                destination[table] = []
                with self.assertRaisesRegex(audit.AuditError,"RESTORE_UNEXPECTED_DELTA"):
                    self.compare(source,destination,expected)
                expected["rows"][table] = {identity:{"before":row,"after":row}}
                with self.assertRaisesRegex(audit.AuditError,"RESTORE_TABLES"):
                    self.compare(source,destination,expected)

    def test_preview_only_dataset_positive_and_late_committed_readback(self):
        source, destination, expected = self.fixture()
        row = {"id":account()["id"],"dataset_id":"x","version":"arbitrary-version",
               "status":"BLOCKED","manifest_sha256":"a"*64,"storage_path":"/data/curated"}
        preview = {key:None for key in DOC["archive_table_columns"]["paper_rebalance_previews"]}
        preview.update(id=account()["id"],dataset_version_id=row["id"],dataset_manifest_sha256=row["manifest_sha256"])
        source["paper_rebalance_previews"] = [(preview["id"],preview)]
        destination["paper_rebalance_previews"] = [(preview["id"],preview)]
        for receipt in expected["receipts"].values():
            receipt["current_epoch"] = 11
            receipt["current_state"] = "RESTORED_OFF"
        self.assertIsNone(self.compare(source,destination,expected,source=[row],after=[row]))
        expected["receipts"]["begin"]["current_epoch"] = Decimal("10.5")
        expected["receipts"]["begin"]["current_state"] = "DRAINING"
        with self.assertRaises(audit.AuditError):
            self.compare(source,destination,expected,source=[row],after=[row])


class MigrationPinTests(unittest.TestCase):
    def test_guard_rejects_unknown_migration_bytes_with_the_complete_source_set(self):
        import re
        root = audit.PROJECTION.parents[2]
        sql = (root / "deploy/db/owner-beta-paper-audit.sql").read_text()
        pins = re.findall(r"\((\d+),'([0-9a-f]{96})'\)", sql)
        expected = [(str(int(path.name.split("_")[0])), hashlib.sha384(path.read_bytes()).hexdigest())
                    for path in sorted((root / "migrations").glob("*.up.sql"))]
        self.assertEqual(pins, expected)
        self.assertIn("version BETWEEN 1 AND 69) <> 69", sql)
        self.assertIn("actual.checksum IS DISTINCT FROM decode(expected.checksum_hex,'hex')", sql)


if __name__ == "__main__":
    unittest.main()
