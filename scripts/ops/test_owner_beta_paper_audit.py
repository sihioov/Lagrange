import copy
import hashlib
import importlib.util
import io
import json
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


if __name__ == "__main__":
    unittest.main()
