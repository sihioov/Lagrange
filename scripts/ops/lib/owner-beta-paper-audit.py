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
