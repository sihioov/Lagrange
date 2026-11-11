"""Strict, descriptive decoder for the fixed Paper database audit stream.

This module never connects to a database, reads referenced files, or returns an
owner-beta-paper-audit-v1 archive. The caller owns snapshot/transport completion,
protected reference traversal and the genuine source_files manifest (P5B).
"""
from __future__ import annotations

import hashlib
import json
import re
from decimal import Decimal, InvalidOperation
from pathlib import Path

PROJECTION = Path(__file__).resolve().parents[3] / "docs/contracts/owner-beta-paper-audit-projection-v1.json"
I64 = 9223372036854775807
MAX_LINE = 146800640  # 64 MiB R4 bytea pair encoded as hex, plus bounded metadata
MAX_DEPTH = 64
MAX_ROWS = 10000000
HASH = re.compile(r"sha256:[0-9a-f]{64}\Z")
ASCII = re.compile(r"[ -~]*\Z")


class AuditError(ValueError):
    """Only fixed codes; neither row values nor parser exception prose escape."""


def require(condition, code):
    if not condition:
        raise AuditError(code)


def digest(value):
    return "sha256:" + hashlib.sha256(value).hexdigest()


def _pairs(items):
    result = {}
    for key, value in items:
        require(key not in result, "DUPLICATE_KEY")
        result[key] = value
    return result


def parse(raw):
    require(type(raw) is bytes and len(raw) <= MAX_LINE, "LINE_SIZE")
    # Bound nesting before the recursive stdlib parser, respecting string escapes.
    depth = 0
    quoted = escaped = False
    for c in raw:
        if quoted:
            if escaped:
                escaped = False
            elif c == 92:
                escaped = True
            elif c == 34:
                quoted = False
        elif c == 34:
            quoted = True
        elif c in (91, 123):
            depth += 1
            require(depth <= MAX_DEPTH, "DEPTH")
        elif c in (93, 125):
            depth -= 1
            require(depth >= 0, "JSON")
    require(depth == 0 and not quoted, "JSON")
    try:
        return json.loads(raw.decode("utf-8"), parse_int=Decimal, parse_float=Decimal,
                          parse_constant=lambda _: (_ for _ in ()).throw(AuditError("NUMBER")),
                          object_pairs_hook=_pairs)
    except (ValueError, UnicodeError, RecursionError, InvalidOperation):
        raise AuditError("JSON") from None


def canonical(value, depth=0):
    require(depth <= MAX_DEPTH, "DEPTH")
    if value is None:
        return b"null"
    if type(value) is bool:
        return b"true" if value else b"false"
    if type(value) in (Decimal, int):
        n = Decimal(value)
        require(n.is_finite() and abs(n.adjusted()) <= 131072 and len(n.as_tuple().digits) <= 131072,
                "NUMBER_SIZE")
        if not n:
            return b"0"
        s = format(n, "f")
        if "." in s:
            s = s.rstrip("0").rstrip(".")
        return s.encode("ascii")
    if type(value) is str:
        try:
            return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
        except UnicodeError:
            raise AuditError("STRING") from None
    if type(value) is list:
        return b"[" + b",".join(canonical(v, depth + 1) for v in value) + b"]"
    if type(value) is dict:
        require(all(type(k) is str for k in value), "KEY_TYPE")
        try:
            ks = sorted(value, key=lambda k: k.encode("utf-8"))
        except UnicodeError:
            raise AuditError("STRING") from None
        return b"{" + b",".join(canonical(k, depth + 1) + b":" + canonical(value[k], depth + 1)
                                 for k in ks) + b"}"
    raise AuditError("TYPE")


def frame(table, key, row):
    require(type(table) is str and type(key) is str and ASCII.fullmatch(table)
            and ASCII.fullmatch(key), "IDENTITY")
    parts = (table.encode("ascii"), key.encode("ascii"), canonical(row))
    return b"".join(str(len(part)).encode("ascii") + b":" + part for part in parts)


class Root:
    def __init__(self):
        self.hash = hashlib.sha256()
        self.count = self.size = 0

    def add(self, data):
        require(self.count < MAX_ROWS and self.size <= I64 - len(data), "OVERFLOW")
        self.hash.update(data)
        self.count += 1
        self.size += len(data)

    def result(self):
        return {"row_count": self.count, "byte_length": self.size,
                "sha256": "sha256:" + self.hash.hexdigest()}


def _receipt(row):
    names = ("request_canonical", "request_sha256", "result_canonical", "result_sha256")
    if "request_canonical" not in row:
        return
    values = [row[n] for n in names]
    require(all(v is None for v in values) or all(type(v) is str for v in values), "RECEIPT_SHAPE")
    if values[0] is None:
        return
    for text, sha in (values[:2], values[2:]):
        require(ASCII.fullmatch(text) is not None and len(text) <= 1048576, "RECEIPT_TEXT")
        raw = text.encode("ascii")
        require(digest(raw) == sha and canonical(parse(raw)) == raw, "RECEIPT_DIGEST")


def _row(table, key, row, a):
    require(type(row) is dict, "ROW")
    if table in a["catalog_projection"]:
        require(set(row) == set(a["catalog_projection"][table]["columns"]), "COLUMNS")
        return
    require(set(row) == set(a["archive_table_columns"][table]), "COLUMNS")
    for c, typ in a["archive_column_types"][table].items():
        v = row[c]
        if v is None:
            continue
        if table == "paper_rebalance_previews" and c == "error_json":
            require(type(v) is dict and set(v) == {"sha256"} and type(v["sha256"]) is str
                    and HASH.fullmatch(v["sha256"]), "ERROR_DIGEST")
        elif c in ("error_message", "error_detail", "last_error"):
            require(type(v) is str and HASH.fullmatch(v), "ERROR_DIGEST")
        elif typ in ("int2", "int4", "int8"):
            bits = {"int2": 16, "int4": 32, "int8": 64}[typ]
            require(type(v) is Decimal and v == v.to_integral_value()
                    and -(2 ** (bits - 1)) <= v < 2 ** (bits - 1), "INTEGER")
        elif typ == "bool":
            require(type(v) is bool, "BOOLEAN")
        elif typ in ("uuid", "date", "timestamptz", "bytea", "text", "varchar"):
            require(type(v) is str, "STRING")
            patterns = {"uuid": r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}",
                        "date": r"\d{4}-\d{2}-\d{2}",
                        "timestamptz": r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{6}Z",
                        "bytea": r"(?:[0-9a-f]{2})*"}
            if typ in patterns:
                require(re.fullmatch(patterns[typ], v, flags=re.ASCII), "NORMALIZATION")
        elif typ == "numeric" and table in ("accounts", "cash_ledger", "daily_equity", "fills", "orders", "positions"):
            require(type(v) is str and re.fullmatch(r"-?(?:0|[1-9][0-9]*)\.[0-9]{4}", v), "FIXED4")
        elif typ == "numeric":
            require(type(v) is Decimal, "NUMERIC")
        else:
            require(typ == "jsonb", "UNSUPPORTED_COLUMN_TYPE")
    parts = []
    for c in a["archive_identity_key_components"][table]:
        v = row[c]
        require(type(v) in (str, Decimal), "ROW_KEY")
        s = v if type(v) is str else canonical(v).decode("ascii")
        if table == "owner_beta_paper_session_events" and c == "event_seq":
            require(s.isascii() and s.isdigit() and len(s) <= 20, "ROW_KEY")
            s = s.zfill(20)
        parts.append(s)
    require(key == ":".join(parts), "ROW_KEY")
    _receipt(row)


def decode(stream, reference_sink=None):
    """Read binary LF lines; optionally pass each reference to a protected consumer.

    The callback is provisional until this function AND transport/transaction
    completion succeed. No reference list or full history is retained in memory.
    """
    raw_projection = PROJECTION.read_bytes()
    a = json.loads(raw_projection)["archive_audit_v1"]
    groups = a["paper_source_table_projection"]
    expected = [(g, t) for g in sorted(groups) for t in groups[g]]
    index = 0
    roots = {g: Root() for g in groups}
    table_roots = {}
    refs = {t: Root() for t in a["source_reference_tables"]}
    table_root = Root()
    previous = None
    ref_previous = None
    pending_group_end = None
    state = "header"
    for raw in iter(lambda: stream.readline(MAX_LINE + 2), b""):
        require(type(raw) is bytes and len(raw) <= MAX_LINE + 1 and raw.endswith(b"\n"), "TRUNCATED_OR_SIZE")
        rec = parse(raw[:-1])
        require(type(rec) is list and rec and type(rec[0]) is str, "RECORD")
        tag = rec[0]
        if state == "header":
            require(rec == ["header", "owner-beta-paper-db-stream-v1", digest(raw_projection)], "HEADER")
            state = "tables"
        elif state == "tables":
            if pending_group_end:
                require(rec == ["group_end", pending_group_end], "GROUP_END")
                pending_group_end = None
                if index == len(expected):
                    state = "references"
                continue
            require(index < len(expected), "EXTRA_TABLE")
            g, t = expected[index]
            if tag == "table_end":
                require(rec == ["table_end", g, t], "TABLE_END")
                result = table_root.result()
                if t in table_roots:
                    require(table_roots[t] == result, "SHARED_TABLE_CHANGED")
                table_roots[t] = result
                table_root = Root()
                previous = None
                index += 1
                if index == len(expected) or expected[index][0] != g:
                    pending_group_end = g
            else:
                require(len(rec) == 5 and rec[:3] == ["row", g, t], "ROW_ORDER")
                key, row = rec[3:]
                require(type(key) is str and ASCII.fullmatch(key) and (previous is None or previous.encode("utf-8") < key.encode("utf-8")), "KEY_ORDER")
                _row(t, key, row, a)
                data = frame(t, key, row)
                table_root.add(data)
                roots[g].add(data)
                previous = key
        elif state == "references":
            if rec == ["references_end"]:
                require(all(refs[t].result() == table_roots[t] for t in refs), "REFERENCE_SET")
                state = "end"
            else:
                require(len(rec) == 4 and tag == "reference" and type(rec[1]) is str
                        and rec[1] in refs and type(rec[2]) is str and ASCII.fullmatch(rec[2]), "REFERENCE")
                _, t, key, row = rec
                identity = (t.encode("utf-8"), key.encode("utf-8"))
                require(ref_previous is None or ref_previous < identity, "REFERENCE_ORDER")
                _row(t, key, row, a)
                refs[t].add(frame(t, key, row))
                if reference_sink is not None:
                    reference_sink(t, key, row)
                ref_previous = identity
        elif state == "end":
            require(rec == ["end"], "END")
            state = "done"
        else:
            raise AuditError("TRAILING_DATA")
    require(state == "done", "INCOMPLETE")
    return {"schema": "owner-beta-paper-database-description-v1",
            "projection_sha256": digest(raw_projection),
            "groups": [{"name": g, **roots[g].result()} for g in sorted(roots)],
            "reference_tables": [{"table": t, **refs[t].result()} for t in sorted(refs)],
            "source_files": "REQUIRED_EXTERNAL_EVIDENCE"}


# The expected rows are captured from the destination's committed exact
# readbacks, not wildcard exclusions. This comparator returns no authority.
_RESTORE_INSERTS = frozenset(("owner_beta_paper_operations", "owner_beta_paper_events",
                             "owner_beta_paper_operation_observations"))
_RESTORE_UPDATES = {
    "owner_beta_paper_control": frozenset(("state", "control_epoch", "process_binding",
        "process_binding_sha256", "cohort_sha256", "daily_cohort", "activation_event_id", "updated_at")),
    "owner_beta_paper_recovery_grants": frozenset(("revoked_at",)),
}


def _restore_changes(expected):
    require(type(expected) is dict and set(expected) == {
        "generation", "installation_id", "account_id", "source_generation", "source_installation_id",
        "source_epoch", "source_control_epoch", "expected_epoch", "destination_epoch_floor",
        "fence_epoch", "restore_epoch", "archive_sha256", "source_audit_sha256", "receipts", "rows"}, "RESTORE_EXPECTED")
    expected = parse(canonical(expected))
    generation, account = expected["generation"], expected["account_id"]
    uuid = r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}"
    require(all(type(v) is str and re.fullmatch(uuid, v) for v in (generation, account, expected["installation_id"], expected["source_installation_id"], expected["source_generation"])), "RESTORE_IDENTITY")
    source, fence, restored = [expected[k] for k in ("source_control_epoch", "fence_epoch", "restore_epoch")]
    require(all(type(v) is Decimal and v == v.to_integral_value() and 0 <= v <= I64
                for v in (source, fence, restored)) and source < fence and restored == fence + 1,
            "RESTORE_EPOCH")
    require(expected["source_epoch"] + 1 == source and expected["expected_epoch"] == source
            and expected["source_installation_id"] != expected["installation_id"]
            and expected["source_generation"] != generation
            and type(expected["destination_epoch_floor"]) is Decimal
            and expected["destination_epoch_floor"] == expected["destination_epoch_floor"].to_integral_value()
            and source <= expected["destination_epoch_floor"] < I64
            and fence == max(source, expected["destination_epoch_floor"]) + 1, "RESTORE_EPOCH")
    require(all(type(expected[k]) is str and HASH.fullmatch(expected[k])
                for k in ("archive_sha256", "source_audit_sha256")), "RESTORE_DIGEST")
    changes = expected["rows"]
    require(type(changes) is dict and set(changes) <= _RESTORE_INSERTS | _RESTORE_UPDATES.keys()
            and _RESTORE_INSERTS | {"owner_beta_paper_control"} <= changes.keys(), "RESTORE_TABLES")
    projection = json.loads(PROJECTION.read_bytes())["archive_audit_v1"]
    count = 0
    for table, rows in changes.items():
        require(type(rows) is dict and rows, "RESTORE_ROWS")
        count += len(rows)
        require(count <= 1_000_000, "RESTORE_LIMIT")
        for key, change in rows.items():
            require(type(change) is dict and set(change) == {"before", "after"}, "RESTORE_CHANGE")
            before, after = change["before"], change["after"]
            _row(table, key, after, projection)
            require((after.get("account_id") if table != "owner_beta_paper_operation_observations"
                     else after["summary"].get("account_id")) == account, "RESTORE_ACCOUNT")
            if table in _RESTORE_INSERTS:
                require(before is None, "RESTORE_INSERT")
            else:
                _row(table, key, before, projection)
                require(before != after and before.get("account_id") == account
                        and all(before[k] == after[k] for k in before if k not in _RESTORE_UPDATES[table]),
                        "RESTORE_FIELDS")
            if table == "owner_beta_paper_control":
                require(key == "etf11" and before["control_epoch"] == source
                        and before["state"] in ("OFF", "RESTORED_OFF")
                        and after["control_epoch"] == restored and after["state"] == "RESTORED_OFF"
                        and all(after[k] is None for k in ("process_binding", "process_binding_sha256",
                            "cohort_sha256", "daily_cohort", "activation_event_id")), "RESTORE_CONTROL")
            elif table == "owner_beta_paper_recovery_grants":
                require(before["revoked_at"] is None and before["valuation_receipt_sha256"] is None
                        and after["revoked_at"] is not None, "RESTORE_GRANT")
            elif table in ("owner_beta_paper_operations", "owner_beta_paper_operation_observations"):
                require(after["generation"] == generation and after["request_canonical"] is not None,
                        "RESTORE_GENERATION")
            elif table == "owner_beta_paper_events":
                require((after["kind"], after["control_epoch"]) in
                        (("DRAIN_STARTED", fence), ("RESTORED_OFF", restored)), "RESTORE_EVENT")
                require(after["payload"].get("generation") == generation, "RESTORE_GENERATION")
    require(len(changes["owner_beta_paper_control"]) == 1
            and len(changes["owner_beta_paper_operations"]) == 1
            and len(changes["owner_beta_paper_operation_observations"]) == 1
            and len(changes["owner_beta_paper_events"]) == 2, "RESTORE_CARDINALITY")
    operation = next(iter(changes["owner_beta_paper_operations"].values()))["after"]
    require(operation["generation"] == generation and operation["purpose"] == "RESTORE"
            and operation["scope"] == "COMBINED_BACKUP" and operation["expected_epoch"] == source
            and operation["fence_epoch"] == fence
            and max(source, operation["local_epoch_floor"]) + 1 == fence, "RESTORE_OPERATION")
    control = changes["owner_beta_paper_control"]["etf11"]["before"]
    require(operation["prior_state"] == control["state"], "RESTORE_OPERATION")
    events = {change["after"]["kind"]: change["after"]
              for change in changes["owner_beta_paper_events"].values()}
    require(set(events) == {"DRAIN_STARTED", "RESTORED_OFF"}
            and events["DRAIN_STARTED"]["id"] == operation["fence_event_id"]
            and events["DRAIN_STARTED"]["payload"] == operation["intent"], "RESTORE_EVENT")
    require(operation["installation_id"] == expected["installation_id"]
            and operation["local_epoch_floor"] == expected["destination_epoch_floor"]
            and operation["intent"].get("predecessor_generation") == expected["source_generation"]
            and operation["intent"].get("artifact_sha256") == expected["archive_sha256"], "RESTORE_OPERATION")
    observation = next(iter(changes["owner_beta_paper_operation_observations"].values()))["after"]
    require(observation["summary"].get("installation_id") == expected["installation_id"]
            and observation["summary"].get("generation") == generation
            and observation["summary"].get("control_epoch") == fence, "RESTORE_OBSERVATION")
    evidence = events["RESTORED_OFF"]["payload"]
    for key in ("installation_id", "generation", "account_id", "source_generation", "source_installation_id",
                "source_epoch", "destination_epoch_floor", "archive_sha256", "source_audit_sha256"):
        require(evidence.get(key) == expected[key], "RESTORE_EVIDENCE")
    require(evidence.get("restored_audit_sha256") == expected["source_audit_sha256"]
            and evidence.get("expected_epoch") == source and evidence.get("resulting_epoch") == fence
            and evidence.get("observation_id") == observation["observation_id"], "RESTORE_EVIDENCE")
    receipts = expected["receipts"]
    require(type(receipts) is dict and set(receipts) == {"begin", "reconcile", "restore-off"}, "RESTORE_RECEIPTS")
    for name, row, kind, event_id, epoch in (
        ("begin", operation, "BEGIN", operation["fence_event_id"], fence),
        ("reconcile", observation, "RECONCILE", observation["observation_id"], fence),
        ("restore-off", events["RESTORED_OFF"], "TRANSITION", events["RESTORED_OFF"]["id"], restored)):
        receipt = receipts[name]
        require(type(receipt) is dict and set(receipt) == {"schema", "kind", "request_sha256", "status",
                "account_id", "current_epoch", "current_state", "event_id", "event_epoch",
                "result_canonical", "result_sha256", "reset"}, "RESTORE_RECEIPT")
        require(receipt["schema"] == "owner-beta-paper-exact-readback-v1" and receipt["kind"] == kind
                and receipt["status"] == "COMMITTED" and receipt["account_id"] == account
                and receipt["event_id"] == event_id and receipt["event_epoch"] == epoch
                and type(receipt["current_epoch"]) is Decimal
                and receipt["current_epoch"] == receipt["current_epoch"].to_integral_value()
                and epoch <= receipt["current_epoch"] <= restored
                and receipt["current_state"] == ("RESTORED_OFF" if receipt["current_epoch"] == restored else "DRAINING")
                and receipt["reset"] is None
                and all(receipt[k] == row[k] for k in ("request_sha256", "result_canonical", "result_sha256")),
                "RESTORE_RECEIPT")
        request = parse(row["request_canonical"].encode("ascii"))
        result = parse(row["result_canonical"].encode("ascii"))
        require(request.get("schema") == "owner-beta-paper-exact-request-v1"
                and request.get("kind") == kind and request.get("account_id") == account
                and request["args"].get("request_key") == row["request_key"]
                and result.get("account_id") == account and result.get("control_epoch") == epoch,
                "RESTORE_RECEIPT")
        require(set(request) == {"schema", "kind", "account_id", "args"}
                and set(result) == {"schema", "action", "outcome", "account_id", "control_epoch", "data"}
                and result["schema"] == "owner-beta-paper-action-v1" and result["outcome"] == "OK",
                "RESTORE_RECEIPT")
        if name == "begin":
            args = {"expected_epoch": source, "intent_canonical": canonical(operation["intent"]).decode("ascii"),
                    "request_key": operation["request_key"]}
            data = {"generation": generation, "operation_event_id": operation["fence_event_id"],
                    "intent_sha256": digest(canonical(operation["intent"])), "fenced_epoch": fence}
            action = "BEGIN_OPERATION"
            require(operation["intent_sha256"] == data["intent_sha256"], "RESTORE_OPERATION")
        elif name == "reconcile":
            args = {"generation": generation, "expected_epoch": fence,
                    "inventory_sha256": observation["summary"].get("inventory_sha256"),
                    "request_key": observation["request_key"]}
            data = {"observation_id": observation["observation_id"], "summary": observation["summary"],
                    "summary_sha256": digest(canonical(observation["summary"]))}
            action = "RECONCILE_OPERATION"
            require(observation["summary_sha256"] == data["summary_sha256"]
                    and type(args["inventory_sha256"]) is str and HASH.fullmatch(args["inventory_sha256"]),
                    "RESTORE_OBSERVATION")
        else:
            args = {"expected_epoch": fence, "transition": "RESTORED_OFF",
                    "evidence_canonical": canonical(evidence).decode("ascii"), "request_key": row["request_key"]}
            data = {"event_id": row["id"], "state": "RESTORED_OFF"}
            action = "TRANSITION"
        require(request["args"] == args and result["action"] == action and result["data"] == data,
                "RESTORE_RECEIPT")
    for event in events.values():
        require(event["payload_sha256"] == digest(canonical(event["payload"]))
                and event["owner_user_id"] == control["owner_user_id"]
                and event["actor_user_id"] is None and event["pending_target_id"] is None,
                "RESTORE_EVENT")
    require(events["DRAIN_STARTED"]["request_key"] == operation["request_key"]
            and all(events["DRAIN_STARTED"][key] is None for key in
                    ("request_canonical", "request_sha256", "result_canonical", "result_sha256")), "RESTORE_EVENT")
    return changes


def _compare_restore_delta(source_stream, after_stream, expected, *, source_dataset_rows, after_dataset_rows):
    """Compare complete validated streams with only the exact destination delta.

    expected carries generation/account/source-control/fence/final epochs plus
    rows[table][identity]={before,after}, all normalized exactly as the SQL audit.
    Every insert and changed field must match the actual committed readback.
    Economic, job, session, outbox, delivery and schema rows have no exclusions.
    """
    changes = _restore_changes(expected)
    dataset_keys = {"id", "dataset_id", "version", "status", "manifest_sha256", "storage_path"}
    def dataset_capture(rows):
        result = {}
        size = 0
        previous = None
        for row in rows:
            require(type(row) is dict and set(row) == dataset_keys
                    and all(type(v) is str for v in row.values()), "RESTORE_DATASET_ROW")
            identity = row["id"]
            require(re.fullmatch(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}", identity)
                    and (previous is None or previous < identity)
                    and re.fullmatch(r"[0-9a-f]{64}", row["manifest_sha256"]), "RESTORE_DATASET_ROW")
            raw = canonical(row)
            size += len(raw) + 1
            require(len(raw) + 1 <= 2 * 1024 * 1024 and size <= 64 * 1024 * 1024
                    and len(result) < 1_000_000, "RESTORE_DATASET_LIMIT")
            result[identity] = row
            previous = identity
        return result
    source_datasets, after_datasets = dataset_capture(source_dataset_rows), dataset_capture(after_dataset_rows)
    require(source_datasets == after_datasets, "RESTORE_DATASET_DELTA")

    def normalize(stream, source):
        roots = {}
        seen = set()
        dataset_ids = set()

        class ReadThrough:
            def readline(self, maximum):
                raw = stream.readline(maximum)
                if type(raw) is bytes and raw.endswith(b"\n") and len(raw) <= maximum:
                    record = parse(raw[:-1])
                    if type(record) is list and len(record) == 5 and record[0] == "row":
                        _, group, table, key, row = record
                        require(type(group) is str and type(table) is str and type(key) is str, "RESTORE_ROW")
                        if table in ("pending_targets", "paper_rebalance_previews"):
                            identity = row.get("dataset_version_id")
                            if identity is not None:
                                dataset_ids.add(identity)
                                captured = (source_datasets if source else after_datasets).get(identity)
                                require(captured is not None and captured["manifest_sha256"] == row.get("dataset_manifest_sha256")
                                        and (row.get("dataset_version") is None or captured["version"] == row["dataset_version"]),
                                        "RESTORE_DATASET_BINDING")
                        change = changes.get(table, {}).get(key)
                        if (source and table == "owner_beta_paper_recovery_grants"
                                and row.get("account_id") == expected["account_id"]
                                and row.get("revoked_at") is None and row.get("valuation_receipt_sha256") is None):
                            require(change is not None, "RESTORE_GRANT")
                        if change is not None:
                            expected_row = change["before"] if source else change["after"]
                            require(row == expected_row, "RESTORE_EXACT_ROW")
                            seen.add((table, key))
                            # Compare the unchanged source projection after
                            # removing precisely the committed insertions and
                            # replacing precisely the committed updated fields.
                            row = change["before"]
                        if row is not None:
                            roots.setdefault((group, table), Root()).add(frame(table, key, row))
                return raw

        decode(ReadThrough())  # validates columns, duplicate groups/refs, EOF
        required = {(table, key) for table, rows in changes.items() for key, value in rows.items()
                    if not source or value["before"] is not None}
        require(seen == required, "RESTORE_EXPECTED_SET")
        require(dataset_ids == (source_datasets if source else after_datasets).keys(), "RESTORE_DATASET_SET")
        return {key: root.result() for key, root in roots.items()}

    before = normalize(source_stream, True)
    after = normalize(after_stream, False)
    require(before == after, "RESTORE_UNEXPECTED_DELTA")


def compare_restore_delta(source_stream, after_stream, expected, *, source_dataset_rows, after_dataset_rows):
    """Exact scoped post-restore comparison; all failures have fixed diagnostics."""
    try:
        return _compare_restore_delta(source_stream, after_stream, expected,
            source_dataset_rows=source_dataset_rows, after_dataset_rows=after_dataset_rows)
    except AuditError:
        raise
    except (ValueError, TypeError, KeyError, AttributeError, UnicodeError, OverflowError, OSError):
        raise AuditError("RESTORE_EXPECTED") from None
