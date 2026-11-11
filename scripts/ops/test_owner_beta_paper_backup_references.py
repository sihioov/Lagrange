"""Pure stream/graph vectors. Real protected FD tests live in the Root suite."""
import copy
import datetime
from decimal import Decimal
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import unittest

sys.dont_write_bytecode = True
PATH = Path(__file__).parent / "lib/owner-beta-paper-backup-references.py"
spec = importlib.util.spec_from_file_location("paper_backup_references_tested", PATH)
ref = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = ref
spec.loader.exec_module(ref)
DOC = json.loads(ref.audit.PROJECTION.read_bytes())["archive_audit_v1"]
A = "11111111-1111-1111-1111-111111111111"
B = "22222222-2222-2222-2222-222222222222"
C = "33333333-3333-3333-3333-333333333333"
D = "44444444-4444-4444-4444-444444444444"
H = "sha256:" + "a" * 64
J = "sha256:" + "b" * 64


def independent_frame(table, key, row):
    # Does not call production canonical/frame helpers.
    data = json.dumps(row, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    return b"".join(str(len(p)).encode("ascii") + b":" + p
                    for p in (table.encode("ascii"), key.encode("ascii"), data))


def limits(entries=20000, manifest=2 * 1024 * 1024):
    return ref.files.Limits(64 * 1024 * 1024, entries, manifest)


def rows_stream(rows=None):
    rows = rows or {}
    records = [["header", "owner-beta-paper-db-stream-v1", "sha256:" + hashlib.sha256(ref.audit.PROJECTION.read_bytes()).hexdigest()]]
    for group, tables in sorted(DOC["paper_source_table_projection"].items()):
        for table in tables:
            records.extend(["row", group, table, key, value] for key, value in rows.get(table, []))
            records.append(["table_end", group, table])
        records.append(["group_end", group])
    for table in DOC["source_reference_tables"]:
        records.extend(["reference", table, key, value] for key, value in rows.get(table, []))
    records += [["references_end"], ["end"]]
    # Fixture integers are normal ints; the real decoder converts them to Decimal.
    return b"".join(json.dumps(r, ensure_ascii=False, separators=(",", ":")).encode() + b"\n" for r in records)


def db_row(table, **values):
    value = {key: None for key in DOC["archive_table_columns"][table]}
    value.update(values)
    return value


def account():
    return db_row("accounts", id=A, owner_user_id=B, account_type="PAPER", name="가😀",
                  currency="KRW", status="ACTIVE", initial_cash="1.0000", paper_state_version=0)


def raw_batch(batch=A, body=b"opaque body", copied=None):
    item = {"kind": "bars", "file_name": "bars.json", "content_hash": "sha256:" + hashlib.sha256(body).hexdigest(),
            "size_bytes": len(body), "request": {"endpoint": "fixture:read-only", "query": [], "headers": [], "mode": "synthetic"}}
    if copied is not None:
        item["copied_from"] = copied
    return {"batch_id": batch, "provider": "kis", "market": "kr", "date": "2026-11-06",
            "retrieved_at": "2026-11-06T07:30:00Z", "mode": "synthetic", "entitlement_reference": None, "files": [item]}


def copy_reference(batch):
    item = batch["files"][0]
    return {"provider": batch["provider"], "market": batch["market"], "batch_id": batch["batch_id"],
            "file_name": item["file_name"], "content_hash": item["content_hash"], "retrieved_at": batch["retrieved_at"]}


def index_batch(index, batch):
    prefix = "raw/provider=kis/market=kr/date=2026-11-06/batch=" + batch["batch_id"]
    return index.add(batch, prefix, H, J)


def daily_calendar_fixture():
    """Actual source wire contracts, deliberately no complete P5 authority."""
    body = b'{"rt_cd":"0","output":[{"bass_dt":"20261106","opnd_yn":"Y"},{"bass_dt":"20261107","opnd_yn":"N"}]}'
    request = {"endpoint": "/uapi/domestic-stock/v1/quotations/chk-holiday",
               "query": [["BASS_DT", "20261106"], ["CTX_AREA_FK", ""], ["CTX_AREA_NK", ""]],
               "headers": [["appkey", "[REDACTED]"], ["appsecret", "[REDACTED]"],
                           ["authorization", "[REDACTED]"], ["tr_cont", ""], ["tr_id", "CTCA0903R"]],
               "mode": "credentialed"}
    digest = lambda b: "sha256:" + hashlib.sha256(b).hexdigest()
    batch = {"batch_id": A, "provider": "kis", "market": "kr", "date": "2026-11-06",
             "retrieved_at": "2026-11-06T07:30:00Z", "mode": "credentialed", "entitlement_reference": "fixture-rights",
             "files": [{"kind": "calendar", "file_name": "calendar.json", "content_hash": digest(body),
                        "size_bytes": len(body), "request": request}]}
    batch_bytes = json.dumps(batch, indent=2).encode() + b"\n"
    line = json.dumps(batch, separators=(",", ":")).encode() + b"\n"
    prefix = "raw/provider=kis/market=kr/date=2026-11-06/batch=" + A
    original = {"provider": "kis", "market": "kr", "batch_id": A, "batch_date": "2026-11-06",
                "batch_json_sha256": digest(batch_bytes), "manifest_line_sha256": digest(line[:-1]),
                "file_name": "calendar.json", "kind": "calendar", "wire_sha256": digest(body),
                "size_bytes": len(body), "request": request, "response_continuation": None,
                "retrieved_at": "2026-11-06T07:30:00.000000000Z", "retrieval_precision": "batch",
                "original_retrieved_at": "2026-11-06T07:30:00.000000000Z", "original_retrieval_precision": "batch", "copied_from": []}
    index = ref._RawIndex(ref._Budget(limits()))
    index.add(batch, prefix, digest(batch_bytes), digest(line[:-1]), 1)
    index.close()
    return {"body": body, "batch": batch, "batch_bytes": batch_bytes, "line": line,
            "prefix": prefix, "file": original, "index": index}


def calendar_rows(fixture):
    file = fixture["file"]
    source_wire = ("owner-beta-daily-calendar-source-v1\n" + B + "\n" + H + "\ndaily-raw-reconstruction-v1\n").encode() + json.dumps(file, separators=(",", ":")).encode()
    identity = "sha256:" + hashlib.sha256(source_wire).hexdigest()
    source = db_row("owner_beta_daily_calendar_sources", calendar_evidence_sha256=identity, owner_user_id=B,
        policy_sha256=H, normalizer_version="daily-raw-reconstruction-v1", source_provider="kis", source_market="kr",
        source_batch_id=A, source_batch_date="2026-11-06", requested_target="2026-11-06", source_file_name="calendar.json",
        source_file_sha256=file["wire_sha256"], source_manifest_line_sha256=file["manifest_line_sha256"],
        source_retrieved_at="2026-11-06T07:30:00.000000Z", copy_chain=[], evidence_json=file,
        projected_at="2026-11-06T07:31:00.000000Z")
    days = []
    for ordinal, (date, kind, evidence) in enumerate((("2026-11-06", "TRADING", "REQUESTED_TARGET"), ("2026-11-07", "CLOSED", "DERIVED_SURROUNDING"))):
        provider = json.loads(fixture["body"])["output"][ordinal]
        row = db_row("owner_beta_daily_calendar_days", owner_user_id=B, calendar_evidence_sha256=identity,
            session_date=date, session_type=kind, evidence_kind=evidence, source_row_ordinal=ordinal,
            source_row_sha256="sha256:" + hashlib.sha256(json.dumps(provider, sort_keys=True, separators=(",", ":")).encode()).hexdigest())
        days.append((B + ":" + identity + ":" + date, row))
    return {"owner_beta_daily_calendar_sources": [(identity, source)], "owner_beta_daily_calendar_days": days}


def calendar_document_fixture(file, date, body, evidence_kind=None):
    """Independent source-defined document bytes; no production hashing helpers."""
    rows = json.loads(body)["output"]
    if isinstance(rows, dict):
        rows = [rows]
    row = next(row for row in rows if row["bass_dt"] == date.replace("-", ""))
    target = file["copied_from"][-1]["batch_date"] if file["copied_from"] else file["batch_date"]
    key = {k: file[k] for k in ("provider", "market", "batch_id", "file_name", "wire_sha256")}
    source = {"schema": "owner-beta-daily-normalized-source-v1", "kind": "calendar", "session_date": date, "source_files": [key]}
    source_sha = "sha256:" + hashlib.sha256(json.dumps(source, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()
    document = {"schema": "owner-beta-daily-reconstructed-document-v1", "normalizer_version": "daily-raw-reconstruction-v1",
        "kind": "calendar", "session_date": date, "source_evidence_sha256": source_sha,
        "rows": [{"date": date, "session_type": "TRADING" if row["opnd_yn"] == "Y" else "CLOSED",
                  "evidence_kind": evidence_kind or ("REQUESTED_TARGET" if date == target else "DERIVED_SURROUNDING")}]}
    encoded = json.dumps(document, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()
    descriptor = {"kind": "calendar", "session_date": date, "normalizer_version": "daily-raw-reconstruction-v1",
        "sha256": "sha256:" + hashlib.sha256(encoded).hexdigest(), "size_bytes": len(encoded), "row_count": 1,
        "source_files": [key], "source_evidence_sha256": source_sha}
    return encoded, descriptor


def reseal_nested_fixture(originals, manifest, admission):
    """Rebind every outer fixture claim so mutations reach the original check."""
    evidence = {"schema": "owner-beta-daily-etf11-evidence-v1", "daily_sources": manifest["daily_sources"]}
    digest = "sha256:" + hashlib.sha256(json.dumps(evidence, separators=(",", ":")).encode()).hexdigest()
    admission["evidence_sha256"] = digest
    documents = [document for session in manifest["daily_sources"] for document in session["normalized_documents"]]
    proof = {"schema": "owner-beta-daily-etf11-normalization-v1", "normalizer_version": "daily-raw-reconstruction-v1",
             "evidence_sha256": digest, "documents": documents}
    originals["daily/admitted/" + H[7:] + "/normalization.json"] = json.dumps(proof, separators=(",", ":")).encode()


def nested_daily_fixture():
    """Nested D4 proof fixture; not a complete historical-base admission."""
    fixture = daily_calendar_fixture()
    batch = fixture["batch"]
    batch["date"], batch["retrieved_at"] = "2026-08-31", "2026-08-31T07:30:00Z"
    calendar_body = b'{"rt_cd":"0","output":[{"bass_dt":"20260829","opnd_yn":"N"},{"bass_dt":"20260830","opnd_yn":"N"},{"bass_dt":"20260831","opnd_yn":"Y"}]}'
    bodies = {"calendar.json": calendar_body}
    calendar = batch["files"][0]
    calendar["request"]["query"][0][1] = "20260831"
    symbols = ("069500", "102110", "114260", "132030", "133690", "143850", "148070", "153130", "192090", "195930", "229200")
    for kind, endpoint, tr in (("bars", "inquire-daily-itemchartprice", "FHKST03010100"), ("reference", "inquire-price", "FHKST01010100")):
        for symbol in symbols:
            item = copy.deepcopy(calendar)
            item["kind"], item["file_name"] = kind, kind + "-" + symbol + ".json"
            item["request"]["endpoint"] = "/uapi/domestic-stock/v1/quotations/" + endpoint
            item["request"]["headers"][-1][1] = tr
            query = {"FID_COND_MRKT_DIV_CODE": "J", "FID_INPUT_ISCD": symbol}
            if kind == "bars": query.update(FID_INPUT_DATE_1="20260831", FID_INPUT_DATE_2="20260831", FID_PERIOD_DIV_CODE="D", FID_ORG_ADJ_PRC="1")
            item["request"]["query"] = [list(pair) for pair in sorted(query.items())]
            bodies[item["file_name"]] = (b'{"rt_cd":"0","output1":{},"output2":[{"stck_bsop_date":"20260831","stck_oprc":"1","stck_hgpr":"1","stck_lwpr":"1","stck_clpr":"1","acml_vol":"1"}]}' if kind == "bars"
                else ('{"rt_cd":"0","output":{"stck_shrn_iscd":"' + symbol + '"}}').encode())
            batch["files"].append(item)
    action_classes = (("paidin-subscription", "paidin-capin", "HHKDB669100C0", "1"), ("paidin-record", "paidin-capin", "HHKDB669100C0", "2"),
        ("bonus-issue", "bonus-issue", "HHKDB669101C0", None), ("dividend", "dividend", "HHKDB669102C0", "0"),
        ("merger-split", "merger-split", "HHKDB669104C0", None), ("reverse-split", "rev-split", "HHKDB669105C0", None),
        ("capital-decrease", "cap-dcrs", "HHKDB669106C0", None))
    for logical, endpoint, tr, gb in action_classes:
        item = copy.deepcopy(calendar)
        item["kind"], item["file_name"] = "corporate_actions", logical + ".json"
        item["request"]["endpoint"] = "/uapi/domestic-stock/v1/ksdinfo/" + endpoint
        item["request"]["headers"][-1][1] = tr
        query = {"CTS": "", "F_DT": "20260829", "T_DT": "20260831", "SHT_CD": ""}
        if gb is not None: query["GB1"] = gb
        if logical == "dividend": query["HIGH_GB"] = ""
        if logical == "reverse-split": query["MARKET_GB"] = "0"
        item["request"]["query"] = [list(pair) for pair in sorted(query.items())]
        bodies[item["file_name"]] = b'{"rt_cd":"0","output1":[]}'
        batch["files"].append(item)
    digest = lambda data: "sha256:" + hashlib.sha256(data).hexdigest()
    encode = lambda value: json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()
    for item in batch["files"]:
        item["content_hash"], item["size_bytes"] = digest(bodies[item["file_name"]]), len(bodies[item["file_name"]])
    batch_bytes, line = json.dumps(batch, indent=2).encode() + b"\n", encode(batch) + b"\n"
    prefix = "raw/provider=kis/market=kr/date=2026-08-31/batch=" + A
    index = ref._RawIndex(ref._Budget(limits()))
    index.add(batch, prefix, digest(batch_bytes), digest(line[:-1]), 1)
    index.close()
    files = {}
    for item in batch["files"]:
        file = copy.deepcopy(fixture["file"])
        file.update(batch_date="2026-08-31", batch_json_sha256=digest(batch_bytes), manifest_line_sha256=digest(line[:-1]),
                    file_name=item["file_name"], kind=item["kind"], wire_sha256=item["content_hash"], size_bytes=item["size_bytes"], request=item["request"],
                    retrieved_at="2026-08-31T07:30:00.000000000Z", original_retrieved_at="2026-08-31T07:30:00.000000000Z")
        files[item["file_name"]] = file
    documents = []
    def document(kind, date, selected, rows):
        keys = [{k: file[k] for k in ("provider", "market", "batch_id", "file_name", "wire_sha256")} for file in selected]
        keys.sort(key=lambda key: tuple(key.values()))
        source = {"schema": "owner-beta-daily-normalized-source-v1", "kind": kind, "session_date": date, "source_files": keys}
        result = {"kind": kind, "session_date": date, "normalizer_version": "daily-raw-reconstruction-v1", "sha256": H,
                  "size_bytes": 100, "row_count": rows, "source_files": keys, "source_evidence_sha256": digest(encode(source))}
        documents.append(result)
        return result
    calendars = []
    for day in ("2026-08-29", "2026-08-30", "2026-08-31"):
        _, doc = calendar_document_fixture(files["calendar.json"], day, calendar_body)
        documents.append(doc)
        calendars.append({"civil_date": day, "session_type": "TRADING" if day.endswith("31") else "CLOSED", "exchange": "KRX",
            "timezone": "Asia/Seoul", "source_file": files["calendar.json"], "normalized_document_sha256": doc["sha256"], "legacy_projection": None})
    for kind in ("bars", "reference", "corporate_actions"):
        document(kind, "2026-08-31", [file for file in files.values() if file["kind"] == kind], 0 if kind == "corporate_actions" else 11)
    documents.sort(key=lambda value: (value["session_date"], value["kind"]))
    actions = [{"logical_class": logical, "scope": "whole_market", "symbol": None, "range_start": "2026-08-29", "range_end": "2026-08-31",
        "pages": [files[logical + ".json"]], "cash_dividends": [], "cash_dividend_rows_sha256": digest(b"[]")} for logical, *_ in sorted(action_classes)]
    session = {"session_date": "2026-08-31", "previous_frontier": "2026-08-28", "instruments": [symbol + ".KRX" for symbol in symbols],
        "bars": [files["bars-" + symbol + ".json"] for symbol in symbols], "reference": [files["reference-" + symbol + ".json"] for symbol in symbols],
        "calendar": calendars, "actions": actions, "normalized_documents": documents}
    evidence = digest(encode({"schema": "owner-beta-daily-etf11-evidence-v1", "daily_sources": [session]}))
    proof = encode({"schema": "owner-beta-daily-etf11-normalization-v1", "normalizer_version": "daily-raw-reconstruction-v1", "evidence_sha256": evidence, "documents": documents})
    manifest = {"instruments": session["instruments"], "sessions": ["2016-08-29", "2026-08-28", "2026-08-31"], "daily_sources": [session],
                "normalization": {"row_count": 6}}
    admission = {"normalizer_version": "daily-raw-reconstruction-v1", "evidence_sha256": evidence, "counts": dict(instrument_count=11,
        session_count=3, bar_count=33, added_session_count=1, source_file_count=30, calendar_row_count=3, action_chain_count=7,
        action_page_count=7, cash_dividend_row_count=0, normalized_document_count=6)}
    originals = {prefix + "/" + name: data for name, data in bodies.items()}
    originals.update({prefix + "/batch.json": batch_bytes, "raw/manifests/provider=kis/market=kr/manifest.jsonl": line,
                      "daily/admitted/" + H[7:] + "/normalization.json": proof})
    return originals, manifest, admission, index


def copied_calendar_fixture(hops=1):
    originals, manifest, admission, index = nested_daily_fixture()
    session = manifest["daily_sources"][0]
    old = session["calendar"][0]["source_file"]
    old_path = "raw/provider=kis/market=kr/date=2026-08-31/batch=" + A + "/calendar.json"
    body = originals[old_path]
    item = {"kind": "calendar", "file_name": "calendar.json", "content_hash": old["wire_sha256"],
        "size_bytes": len(body), "request": copy.deepcopy(old["request"]),
        "copied_from": {"provider": "kis", "market": "kr", "batch_id": A, "file_name": "calendar.json",
                        "content_hash": old["wire_sha256"], "retrieved_at": "2026-08-31T07:30:00Z"}}
    batch = {"batch_id": B, "provider": "kis-calendar", "market": "kr", "date": "2026-09-01",
        "retrieved_at": "2026-09-01T07:30:00Z", "mode": "credentialed", "entitlement_reference": "fixture-rights", "files": [item]}
    batch_bytes, line = json.dumps(batch, indent=2).encode() + b"\n", json.dumps(batch, separators=(",", ":")).encode() + b"\n"
    digest = lambda data: "sha256:" + hashlib.sha256(data).hexdigest()
    prefix = "raw/provider=kis-calendar/market=kr/date=2026-09-01/batch=" + B
    index.add(batch, prefix, digest(batch_bytes), digest(line[:-1]), 1)
    index.close()
    original_fields = ("provider", "market", "batch_id", "batch_date", "batch_json_sha256", "manifest_line_sha256", "file_name",
                       "kind", "wire_sha256", "size_bytes", "request", "response_continuation", "retrieved_at", "retrieval_precision")
    file = copy.deepcopy(old)
    file.update(provider="kis-calendar", batch_id=B, batch_date="2026-09-01", batch_json_sha256=digest(batch_bytes),
                manifest_line_sha256=digest(line[:-1]), retrieved_at="2026-09-01T07:30:00.000000000Z",
                copied_from=[{key: copy.deepcopy(old[key]) for key in original_fields}])
    originals.update({prefix + "/calendar.json": body, prefix + "/batch.json": batch_bytes,
                      "raw/manifests/provider=kis-calendar/market=kr/manifest.jsonl": line})
    if hops == 2:
        prior = copy.deepcopy(file)
        batch = copy.deepcopy(batch)
        batch.update(batch_id=C, date="2026-09-02", retrieved_at="2026-09-02T07:30:00Z")
        batch["files"][0]["copied_from"] = {"provider": "kis-calendar", "market": "kr", "batch_id": B,
            "file_name": "calendar.json", "content_hash": old["wire_sha256"], "retrieved_at": "2026-09-01T07:30:00Z"}
        batch_bytes, line = json.dumps(batch, indent=2).encode() + b"\n", json.dumps(batch, separators=(",", ":")).encode() + b"\n"
        prefix = "raw/provider=kis-calendar/market=kr/date=2026-09-02/batch=" + C
        index.add(batch, prefix, digest(batch_bytes), digest(line[:-1]), 2)
        index.close()
        file.update(batch_id=C, batch_date="2026-09-02", batch_json_sha256=digest(batch_bytes), manifest_line_sha256=digest(line[:-1]),
            retrieved_at="2026-09-02T07:30:00.000000000Z", copied_from=[{key: prior[key] for key in original_fields}, *prior["copied_from"]])
        originals.update({prefix + "/calendar.json": body, prefix + "/batch.json": batch_bytes})
        originals["raw/manifests/provider=kis-calendar/market=kr/manifest.jsonl"] += line
    for claim in session["calendar"]:
        claim["source_file"] = copy.deepcopy(file)
        _, descriptor = calendar_document_fixture(file, claim["civil_date"], body)
        claim["normalized_document_sha256"] = descriptor["sha256"]
        existing = next(d for d in session["normalized_documents"] if d["kind"] == "calendar" and d["session_date"] == claim["civil_date"])
        existing.clear()
        existing.update(descriptor)
    reseal_nested_fixture(originals, manifest, admission)
    return originals, manifest, admission, index


def delivered_calendar_fixture(*, provider="kis-calendar", body=None, selected=(0, 1, 2), batch_date="2026-08-31", query_date="20260831", marker=""):
    """A distinct original delivery, including the provider's object response."""
    originals, manifest, admission, index = nested_daily_fixture()
    session = manifest["daily_sources"][0]
    old = session["calendar"][0]["source_file"]
    if body is None:
        body = originals["raw/provider=kis/market=kr/date=2026-08-31/batch=" + A + "/calendar.json"]
    request = copy.deepcopy(old["request"])
    request["query"][0][1] = query_date
    next(pair for pair in request["headers"] if pair[0] == "tr_cont")[1] = marker
    digest = lambda data: "sha256:" + hashlib.sha256(data).hexdigest()
    item = {"kind": "calendar", "file_name": "calendar.json", "content_hash": digest(body), "size_bytes": len(body), "request": request}
    batch = {"batch_id": B, "provider": provider, "market": "kr", "date": batch_date, "retrieved_at": "2026-08-31T07:31:00Z",
             "mode": "credentialed", "entitlement_reference": "fixture-rights", "files": [item]}
    batch_bytes, line = json.dumps(batch, indent=2).encode() + b"\n", json.dumps(batch, separators=(",", ":")).encode() + b"\n"
    prefix = "raw/provider=" + provider + "/market=kr/date=" + batch_date + "/batch=" + B
    manifest_path = "raw/manifests/provider=" + provider + "/market=kr/manifest.jsonl"
    index.add(batch, prefix, digest(batch_bytes), digest(line[:-1]), originals.get(manifest_path, b"").count(b"\n") + 1)
    index.close()
    file = copy.deepcopy(old)
    file.update(provider=provider, batch_id=B, batch_date=batch_date, batch_json_sha256=digest(batch_bytes), manifest_line_sha256=digest(line[:-1]),
        wire_sha256=digest(body), size_bytes=len(body), request=request, retrieved_at="2026-08-31T07:31:00.000000000Z",
        original_retrieved_at="2026-08-31T07:31:00.000000000Z")
    originals.update({prefix + "/calendar.json": body, prefix + "/batch.json": batch_bytes})
    originals[manifest_path] = originals.get(manifest_path, b"") + line
    for position in selected:
        claim = session["calendar"][position]
        claim["source_file"] = copy.deepcopy(file)
        _, descriptor = calendar_document_fixture(file, claim["civil_date"], body)
        claim["normalized_document_sha256"] = descriptor["sha256"]
        existing = next(d for d in session["normalized_documents"] if d["kind"] == "calendar" and d["session_date"] == claim["civil_date"])
        existing.clear()
        existing.update(descriptor)
    if len(selected) != 3:
        admission["counts"]["source_file_count"] += 1
    reseal_nested_fixture(originals, manifest, admission)
    return originals, manifest, admission, index


class _FixtureBytes:
    """Pure domain byte fixtures; the production entry point rejects this type."""
    def __init__(self, values):
        self.values = values

    def read(self, path, maximum, metadata=None):
        value = self.values[path]
        if len(value) > maximum:
            raise ref.ReferenceError("REFERENCES_DOCUMENT_SIZE")
        return value


class CalendarNormalizationTests(unittest.TestCase):
    def test_integrated_closed_requested_day_and_trading_derived_day(self):
        originals, manifest, admission, index = delivered_calendar_fixture(batch_date="2026-08-29", query_date="20260829")
        ref._daily_nested(_FixtureBytes(originals), "daily/admitted/" + H[7:], manifest, admission, index)
        for claim in manifest["daily_sources"][0]["calendar"]:
            encoded, _ = ref._calendar_document(_FixtureBytes(originals), claim, index)
            row = json.loads(encoded)["rows"][0]
            self.assertEqual(row["evidence_kind"], "REQUESTED_TARGET" if claim["civil_date"] == "2026-08-29" else "DERIVED_SURROUNDING")
            self.assertEqual(row["session_type"], claim["session_type"])

    def test_integrated_two_copy_hops_use_terminal_not_immediate_origin(self):
        originals, manifest, admission, index = copied_calendar_fixture(hops=2)
        for claim in manifest["daily_sources"][0]["calendar"]:
            self.assertEqual([hop["batch_date"] for hop in claim["source_file"]["copied_from"]], ["2026-09-01", "2026-08-31"])
            encoded, _ = ref._calendar_document(_FixtureBytes(originals), claim, index)
            self.assertEqual(json.loads(encoded)["rows"][0]["evidence_kind"],
                             "REQUESTED_TARGET" if claim["civil_date"] == "2026-08-31" else "DERIVED_SURROUNDING")
        ref._daily_nested(_FixtureBytes(originals), "daily/admitted/" + H[7:], manifest, admission, index)

    def test_integrated_kis_calendar_direct_and_single_object_originals(self):
        for arguments in ({}, {"provider": "kis"}, {"body": b'{"rt_cd":"0","output":{"bass_dt":"20260831","opnd_yn":"Y"}}', "selected": (2,)}):
            originals, manifest, admission, index = delivered_calendar_fixture(**arguments)
            ref._daily_nested(_FixtureBytes(originals), "daily/admitted/" + H[7:], manifest, admission, index)

    def test_integrated_request_target_and_continuation_are_original_bound(self):
        for arguments in ({"query_date": "20260830"}, {"marker": "N"}):
            originals, manifest, admission, index = delivered_calendar_fixture(**arguments)
            with self.subTest(arguments=arguments), self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_CALENDAR_REQUEST$"):
                ref._daily_nested(_FixtureBytes(originals), "daily/admitted/" + H[7:], manifest, admission, index)

    def test_integrated_rebound_original_wire_identity_changes_document(self):
        before = nested_daily_fixture()
        body = before[0]["raw/provider=kis/market=kr/date=2026-08-31/batch=" + A + "/calendar.json"]
        value = json.loads(body)
        value["output"][0]["fixture_label"] = "가😀"
        value["output"][0]["fixture_integer"] = 1
        changed = json.dumps(value, ensure_ascii=False, indent=1).encode()
        originals, manifest, admission, index = delivered_calendar_fixture(body=changed)
        ref._daily_nested(_FixtureBytes(originals), "daily/admitted/" + H[7:], manifest, admission, index)
        session = manifest["daily_sources"][0]
        prior_descriptors = {d["session_date"]: d for d in before[1]["daily_sources"][0]["normalized_documents"] if d["kind"] == "calendar"}
        for claim in session["calendar"]:
            descriptor = next(d for d in session["normalized_documents"] if d["kind"] == "calendar" and d["session_date"] == claim["civil_date"])
            old = prior_descriptors[claim["civil_date"]]
            self.assertNotEqual(old["sha256"], descriptor["sha256"])
            # Keep the NEW valid source commitment but transplant the old hash.
            descriptor["sha256"] = claim["normalized_document_sha256"] = old["sha256"]
        reseal_nested_fixture(originals, manifest, admission)
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_CALENDAR_DOCUMENT$"):
            ref._daily_nested(_FixtureBytes(originals), "daily/admitted/" + H[7:], manifest, admission, index)

    def test_existing_source_row_float_rejection_is_preserved(self):
        body = b'{"rt_cd":"0","output":{"bass_dt":"20260831","opnd_yn":"Y","extra":1.0}}'
        originals, manifest, admission, index = delivered_calendar_fixture(body=body, selected=(2,))
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_SOURCE_NUMBER_UNIMPLEMENTED$"):
            ref._daily_nested(_FixtureBytes(originals), "daily/admitted/" + H[7:], manifest, admission, index)

    def test_independent_exact_requested_document_vector(self):
        fixture = daily_calendar_fixture()
        expected = (b'{"kind":"calendar","normalizer_version":"daily-raw-reconstruction-v1",'
            b'"rows":[{"date":"2026-11-06","evidence_kind":"REQUESTED_TARGET","session_type":"TRADING"}],'
            b'"schema":"owner-beta-daily-reconstructed-document-v1","session_date":"2026-11-06",'
            b'"source_evidence_sha256":"sha256:a69a5e548278c4a3b2e427be88756a8ffa0742e74a1454d6c5b2a4085a4cbceb"}')
        _, descriptor = calendar_document_fixture(fixture["file"], "2026-11-06", fixture["body"])
        claim = {"civil_date": "2026-11-06", "session_type": "TRADING", "source_file": fixture["file"],
                 "normalized_document_sha256": descriptor["sha256"], "legacy_projection": None}
        actual, actual_descriptor = ref._calendar_document(
            _FixtureBytes({fixture["prefix"] + "/calendar.json": fixture["body"]}), claim, fixture["index"])
        self.assertEqual(actual, expected)
        self.assertEqual(actual_descriptor, descriptor)
        self.assertEqual(actual_descriptor["size_bytes"], 342)
        self.assertEqual(actual_descriptor["sha256"], "sha256:292d859eebd5fcc5aeed48fa794868f0545019a94c7134d9f31d15d227724d1c")

    def test_integrated_exact_requested_and_derived_documents(self):
        originals, manifest, admission, index = nested_daily_fixture()
        ref._daily_nested(_FixtureBytes(originals), "daily/admitted/" + H[7:], manifest, admission, index)
        for claim in manifest["daily_sources"][0]["calendar"]:
            encoded, descriptor = ref._calendar_document(_FixtureBytes(originals), claim, index)
            value = json.loads(encoded)
            expected_kind = "REQUESTED_TARGET" if claim["civil_date"] == "2026-08-31" else "DERIVED_SURROUNDING"
            self.assertEqual(value["rows"], [{"date": claim["civil_date"], "session_type": claim["session_type"], "evidence_kind": expected_kind}])
            self.assertNotEqual(descriptor["sha256"], H)
            self.assertNotEqual(descriptor["size_bytes"], 100)

    def test_integrated_jointly_forged_calendar_hash_is_rejected(self):
        originals, manifest, admission, index = nested_daily_fixture()
        session = manifest["daily_sources"][0]
        claim = session["calendar"][0]
        descriptor = next(value for value in session["normalized_documents"]
                          if value["kind"] == "calendar" and value["session_date"] == claim["civil_date"])
        claim["normalized_document_sha256"] = descriptor["sha256"] = H
        descriptor["size_bytes"] = 100
        reseal_nested_fixture(originals, manifest, admission)
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_CALENDAR_DOCUMENT$"):
            ref._daily_nested(_FixtureBytes(originals), "daily/admitted/" + H[7:], manifest, admission, index)

    def test_integrated_descriptor_size_count_key_commitment_mutations(self):
        for change in ("size", "count", "source_key", "source_commitment", "proof_only"):
            originals, manifest, admission, index = nested_daily_fixture()
            descriptor = next(value for value in manifest["daily_sources"][0]["normalized_documents"] if value["kind"] == "calendar")
            if change == "size": descriptor["size_bytes"] += 1
            elif change == "count": descriptor["row_count"] = 2
            elif change == "source_key": descriptor["source_files"][0]["batch_id"] = C
            elif change == "source_commitment": descriptor["source_evidence_sha256"] = H
            reseal_nested_fixture(originals, manifest, admission)
            if change == "proof_only":
                path = "daily/admitted/" + H[7:] + "/normalization.json"
                value = json.loads(originals[path])
                next(d for d in value["documents"] if d["kind"] == "calendar")["size_bytes"] += 1
                originals[path] = json.dumps(value, separators=(",", ":")).encode()
            with self.subTest(change=change), self.assertRaises(ref.ReferenceError):
                ref._daily_nested(_FixtureBytes(originals), "daily/admitted/" + H[7:], manifest, admission, index)

    def test_integrated_requested_derived_identity_is_not_chosen_by_claim(self):
        for day, forged_kind in (("2026-08-31", "DERIVED_SURROUNDING"), ("2026-08-29", "REQUESTED_TARGET")):
            originals, manifest, admission, index = nested_daily_fixture()
            session = manifest["daily_sources"][0]
            claim = next(c for c in session["calendar"] if c["civil_date"] == day)
            body = originals["raw/provider=kis/market=kr/date=2026-08-31/batch=" + A + "/calendar.json"]
            _, forged = calendar_document_fixture(claim["source_file"], day, body, forged_kind)
            descriptor = next(d for d in session["normalized_documents"] if d["kind"] == "calendar" and d["session_date"] == day)
            descriptor.update(forged)
            claim["normalized_document_sha256"] = forged["sha256"]
            reseal_nested_fixture(originals, manifest, admission)
            with self.subTest(day=day), self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_CALENDAR_DOCUMENT$"):
                ref._daily_nested(_FixtureBytes(originals), "daily/admitted/" + H[7:], manifest, admission, index)

    def test_integrated_copy_uses_terminal_original_target_and_accepts_legacy_branch(self):
        for legacy in (False, True):
            originals, manifest, admission, index = copied_calendar_fixture()
            for claim in manifest["daily_sources"][0]["calendar"]:
                if legacy:
                    claim["legacy_projection"] = {"source": "kis", "source_version": "kis-chk-holiday-v1:schema-1",
                        "source_batch_id": A, "source_file_name": "calendar.json", "document_sha256": J,
                        "retrieved_at": "2026-08-31T07:30:00.000000000Z"}
                encoded, _ = ref._calendar_document(_FixtureBytes(originals), claim, index)
                self.assertEqual(json.loads(encoded)["rows"][0]["evidence_kind"],
                    "REQUESTED_TARGET" if claim["civil_date"] == "2026-08-31" else "DERIVED_SURROUNDING")
            reseal_nested_fixture(originals, manifest, admission)
            ref._daily_nested(_FixtureBytes(originals), "daily/admitted/" + H[7:], manifest, admission, index)

    def test_integrated_copy_request_and_legacy_substitution_rejected(self):
        for change in ("copy_identity", "copy_date", "original_time", "request", "legacy_batch", "legacy_version", "legacy_time"):
            originals, manifest, admission, index = copied_calendar_fixture()
            claim = manifest["daily_sources"][0]["calendar"][0]
            file = claim["source_file"]
            if change == "copy_identity": file["copied_from"][0]["batch_id"] = C
            elif change == "copy_date": file["copied_from"][0]["batch_date"] = "2026-09-01"
            elif change == "original_time": file["original_retrieved_at"] = "2026-09-01T07:30:00.000000000Z"
            elif change == "request": file["request"]["query"][0][1] = "20260901"
            else:
                claim["legacy_projection"] = {"source": "kis", "source_version": "kis-chk-holiday-v1:schema-1", "source_batch_id": A,
                    "source_file_name": "calendar.json", "document_sha256": J, "retrieved_at": "2026-08-31T07:30:00.000000000Z"}
                field = {"legacy_batch": "source_batch_id", "legacy_version": "source_version", "legacy_time": "retrieved_at"}[change]
                claim["legacy_projection"][field] = {"legacy_batch": B, "legacy_version": "wrong", "legacy_time": "2026-09-01T07:30:00.000000000Z"}[change]
            reseal_nested_fixture(originals, manifest, admission)
            with self.subTest(change=change), self.assertRaises(ref.ReferenceError):
                ref._daily_nested(_FixtureBytes(originals), "daily/admitted/" + H[7:], manifest, admission, index)

    def test_integrated_changed_original_bytes_are_not_replaced_by_claims(self):
        for changed in (b'{}', b'{"rt_cd":"0","output":[]}', None):
            originals, manifest, admission, index = nested_daily_fixture()
            path = "raw/provider=kis/market=kr/date=2026-08-31/batch=" + A + "/calendar.json"
            originals[path] = originals[path].replace(b'"N"', b'"Y"', 1) if changed is None else changed
            with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_CALENDAR_ORIGINAL$"):
                ref._daily_nested(_FixtureBytes(originals), "daily/admitted/" + H[7:], manifest, admission, index)


def poll_receipt(result=None, stage="ADMISSION", **changes):
    row = db_row("owner_beta_daily_poll_receipts", receipt_id=A, stage=stage, owner_user_id=B,
        policy_sha256=H, activation_epoch=1, started_at="2026-11-06T08:00:00.000000Z",
        completed_at=None if result is None else "2026-11-06T08:00:01.000000Z", result_code=result)
    row.update(changes)
    return row


def poll_references(receipts, **tables):
    rows = {"owner_beta_daily_poll_receipts": sorted((r["receipt_id"], r) for r in receipts), **tables}
    if "owner_beta_daily_poll_state" not in rows:
        latest = {}
        for row in sorted(receipts, key=lambda r: r["started_at"]):
            latest[row["stage"]] = row
        rows["owner_beta_daily_poll_state"] = []
        for stage, row in sorted(latest.items()):
            due = datetime.datetime.fromisoformat(row["started_at"].replace("Z", "+00:00")) + datetime.timedelta(seconds=60)
            state = db_row("owner_beta_daily_poll_state", stage=stage, latest_receipt_id=row["receipt_id"],
                next_poll_at=due.strftime("%Y-%m-%dT%H:%M:%S.%fZ"), last_completed_at=row["completed_at"],
                last_result_code=row["result_code"], verified_admission_sha256=row["admission_sha256"],
                source_frontier_sha256=row["source_frontier_sha256"])
            rows["owner_beta_daily_poll_state"].append((stage, state))
    references = ref._References(limits(), DOC)
    references.decode(io.BytesIO(rows_stream(rows)))
    return references


def source_event(evidence=H, kind="BAR", code="SOURCE_CONFLICT", lo="2026-11-06", hi="2026-11-06", owner=B):
    preimage = "owner-beta-daily-source-event-v1\n" + "\n".join((owner, kind, code, lo, hi, evidence)) + "\n"
    identity = "sha256:" + hashlib.sha256(preimage.encode()).hexdigest()
    return db_row("owner_beta_daily_source_events", event_sha256=identity, owner_user_id=owner,
        source_kind=kind, code=code, range_start=lo, range_end=hi, evidence_sha256=evidence,
        recorded_at="2026-11-06T08:00:00.500000Z")


def fixture_jsonb(value):
    """Independent stdlib encoding for fixture strings/integers (no production codec)."""
    def ordered(item):
        if isinstance(item, dict):
            return {key: ordered(item[key]) for key in sorted(item, key=lambda k: (len(k.encode()), k.encode()))}
        if isinstance(item, list):
            return [ordered(v) for v in item]
        return item
    return json.dumps(ordered(value), ensure_ascii=False).encode()


def fixture_digest(value):
    return "sha256:" + hashlib.sha256(value).hexdigest()


def block_receipt(event):
    payload = {k: event[k] for k in ("source_kind", "range_start", "range_end", "code", "evidence_sha256")}
    return poll_receipt("INPUT_BLOCKED", source_frontier_sha256=event["evidence_sha256"],
        event_sha256=event["event_sha256"], completion_binding_sha256=fixture_digest(b"sql-block-v1" + fixture_jsonb(payload)))


def calendar_poll_fixture(*, conflict=True, copies=0, policy=H):
    """Two retained actual Raw originals, one unpublished calendar proposal."""
    fixture = daily_calendar_fixture()
    values = {fixture["prefix"] + "/calendar.json": fixture["body"],
              fixture["prefix"] + "/batch.json": fixture["batch_bytes"],
              "raw/manifests/provider=kis/market=kr/manifest.jsonl": fixture["line"]}
    body = ('{"rt_cd":"0","output":[{"bass_dt":"20261107","opnd_yn":"N"},'
            '{"bass_dt":"20261106","opnd_yn":"' + ("N" if conflict else "Y") + '"}]}').encode()
    batch = copy.deepcopy(fixture["batch"])
    batch.update(batch_id=C, retrieved_at="2026-11-06T07:35:00Z")
    batch["files"][0].update(content_hash=fixture_digest(body), size_bytes=len(body))
    file = copy.deepcopy(fixture["file"])
    for i in range(copies + 1):
        if i:
            previous = copy.deepcopy(file)
            batch["files"][0]["copied_from"] = copy_reference(batch)
            batch.update(batch_id=(D if i == 1 else A), provider="kis-calendar", date="2026-11-07",
                         retrieved_at="2026-11-07T07:35:00Z")
        batch_bytes = json.dumps(batch, indent=2).encode() + b"\n"
        line = json.dumps(batch, separators=(",", ":")).encode() + b"\n"
        prefix = "raw/provider=" + batch["provider"] + "/market=kr/date=" + batch["date"] + "/batch=" + batch["batch_id"]
        manifest = "raw/manifests/provider=" + batch["provider"] + "/market=kr/manifest.jsonl"
        fixture["index"].add(copy.deepcopy(batch), prefix, fixture_digest(batch_bytes), fixture_digest(line[:-1]),
                             values.get(manifest, b"").count(b"\n") + 1)
        file.update(provider=batch["provider"], batch_id=batch["batch_id"], batch_date=batch["date"],
            batch_json_sha256=fixture_digest(batch_bytes), manifest_line_sha256=fixture_digest(line[:-1]),
            wire_sha256=fixture_digest(body), size_bytes=len(body), retrieved_at=batch["retrieved_at"][:-1] + ".000000000Z",
            original_retrieved_at="2026-11-06T07:35:00.000000000Z")
        if i:
            file["copied_from"] = [{k: previous[k] for k in ref._ORIGIN}, *previous["copied_from"]]
        values[prefix + "/calendar.json"], values[prefix + "/batch.json"] = body, batch_bytes
        values[manifest] = values.get(manifest, b"") + line
    fixture["index"].close()
    wire = json.dumps(file, separators=(",", ":")).encode()
    observation = fixture_digest(b"owner-beta-daily-source-observation-v1\n[" + wire + b"]")
    identity = fixture_digest(("owner-beta-daily-calendar-source-v1\n" + B + "\n" + policy
        + "\ndaily-raw-reconstruction-v1\n").encode() + wire)
    days = []
    for ordinal, provider in enumerate(json.loads(body)["output"]):
        date = "2026-11-" + provider["bass_dt"][-2:]
        days.append(dict(session_date=date, session_type="TRADING" if provider["opnd_yn"] == "Y" else "CLOSED",
            evidence_kind="REQUESTED_TARGET" if date == "2026-11-06" else "DERIVED_SURROUNDING", source_row_ordinal=ordinal,
            source_row_sha256=fixture_digest(json.dumps(provider, sort_keys=True, separators=(",", ":")).encode())))
    days.sort(key=lambda row: row["session_date"])
    event = source_event(observation, "CALENDAR", "CALENDAR_CONFLICT", hi="2026-11-07")
    row = poll_receipt("INPUT_BLOCKED", policy_sha256=policy, source_frontier_sha256=observation,
        event_sha256=event["event_sha256"], completion_binding_sha256=fixture_digest(b"sql-calendar-v1" + fixture_jsonb([identity, file, days])))
    if copies:
        row.update(started_at="2026-11-07T08:00:00.000000Z", completed_at="2026-11-07T08:00:01.000000Z")
        event["recorded_at"] = "2026-11-07T08:00:00.500000Z"
    rows = calendar_rows(fixture)
    rows["owner_beta_daily_source_events"] = [(event["event_sha256"], event)]
    return dict(values=values, raw=fixture["index"], row=row, event=event, rows=rows, file=file, days=days,
                identity=identity, path=prefix + "/calendar.json")


class _PollFixtureBytes(_FixtureBytes):
    """Pure original-byte domain fixture; public describe still rejects it."""
    @property
    def entries(self):
        return self.values

    def hash_file(self, path, expected, size, metadata=None):
        value = self.values[path]
        if fixture_digest(value) != expected or len(value) != size:
            raise ref.ReferenceError("REFERENCES_PIN")
        return expected


def fixture_calendar_projection(file, body, policy=H):
    wire = json.dumps(file, separators=(",", ":")).encode()
    identity = fixture_digest(("owner-beta-daily-calendar-source-v1\n" + B + "\n" + policy + "\ndaily-raw-reconstruction-v1\n").encode() + wire)
    origin = file["copied_from"][-1] if file["copied_from"] else file
    source = db_row("owner_beta_daily_calendar_sources", calendar_evidence_sha256=identity, owner_user_id=B,
        policy_sha256=policy, normalizer_version="daily-raw-reconstruction-v1", source_provider=origin["provider"],
        source_market=origin["market"], source_batch_id=origin["batch_id"], source_batch_date=origin["batch_date"],
        requested_target=origin["batch_date"], source_file_name=origin["file_name"], source_file_sha256=origin["wire_sha256"],
        source_manifest_line_sha256=origin["manifest_line_sha256"], source_retrieved_at=origin["retrieved_at"][:-4] + "Z",
        copy_chain=file["copied_from"], evidence_json=file, projected_at="2026-11-06T07:40:00.000000Z")
    rows = json.loads(body)["output"]
    if isinstance(rows, dict): rows = [rows]
    days = []
    for ordinal, raw in enumerate(rows):
        date = raw["bass_dt"][:4] + "-" + raw["bass_dt"][4:6] + "-" + raw["bass_dt"][6:]
        day = db_row("owner_beta_daily_calendar_days", owner_user_id=B, calendar_evidence_sha256=identity,
            session_date=date, session_type="TRADING" if raw["opnd_yn"] == "Y" else "CLOSED",
            evidence_kind="REQUESTED_TARGET" if date == origin["batch_date"] else "DERIVED_SURROUNDING",
            source_row_ordinal=ordinal, source_row_sha256=fixture_digest(json.dumps(raw, sort_keys=True, separators=(",", ":")).encode()))
        days.append((B + ":" + identity + ":" + date, day))
    return {"owner_beta_daily_calendar_sources": [(identity, source)], "owner_beta_daily_calendar_days": sorted(days)}


def admission_poll_fixture(*, result="VERIFIED", verifier="2", batch_id=A):
    """D4's source-test miniature base, never an approved 2452-session base.

    Matches daily_price_only.rs make_draft/fixture's three-date shape and marked
    synthetic bars. This tests retained transport/observation integrity, not the
    financial calculator or SQL admission's production-base acceptance.
    """
    values, small, partial, raw = nested_daily_fixture()
    encode = lambda value: json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()
    if batch_id != A:
        prefix = "raw/provider=kis/market=kr/date=2026-08-31/batch="
        batch = json.loads(values[prefix + A + "/batch.json"])
        batch["batch_id"] = batch_id
        batch_bytes, line = json.dumps(batch, indent=2).encode() + b"\n", encode(batch) + b"\n"
        values = {path.replace(prefix + A, prefix + batch_id): data for path, data in values.items()}
        values[prefix + batch_id + "/batch.json"] = batch_bytes
        values["raw/manifests/provider=kis/market=kr/manifest.jsonl"] = line
        raw = ref._RawIndex(ref._Budget(limits()))
        raw.add(batch, prefix + batch_id, fixture_digest(batch_bytes), fixture_digest(line[:-1]), 1)
        raw.close()
        def rebind(value):
            if isinstance(value, dict):
                if "batch_id" in value: value["batch_id"] = batch_id
                if "batch_json_sha256" in value:
                    value.update(batch_json_sha256=fixture_digest(batch_bytes), manifest_line_sha256=fixture_digest(line[:-1]))
                for child in value.values(): rebind(child)
            elif isinstance(value, list):
                for child in value: rebind(child)
        rebind(small)
        for session in small["daily_sources"]:
            for document in session["normalized_documents"]:
                source = dict(schema="owner-beta-daily-normalized-source-v1", kind=document["kind"],
                    session_date=document["session_date"], source_files=document["source_files"])
                document["source_evidence_sha256"] = fixture_digest(encode(source))
            for claim in session["calendar"]:
                _, descriptor = calendar_document_fixture(claim["source_file"], claim["civil_date"], values[prefix + batch_id + "/calendar.json"])
                claim["normalized_document_sha256"] = descriptor["sha256"]
                target = next(d for d in session["normalized_documents"] if d["kind"] == "calendar" and d["session_date"] == claim["civil_date"])
                target.clear(); target.update(descriptor)
        reseal_nested_fixture(values, small, partial)
    proof = values.pop("daily/admitted/" + H[7:] + "/normalization.json")
    bars = b"synthetic-bars-not-approved"
    base = dict(kind="historical_price_only_v3", pins=dict(candidate_content_sha256=H, artifact_manifest_sha256=J,
        stage5_manifest_sha256=H, action_manifest_sha256=J, approval_registry_sha256=H))
    manifest = dict(schema="owner-beta-daily-etf11-input-v1", capability="PRICE_RETURN_ONLY", audience="OWNER_ONLY",
        vendor_snapshot=True, strict_pit=False, base=base, parent_admission_sha256=None, range_start="2016-08-29",
        range_end="2026-08-31", instruments=small["instruments"], sessions=small["sessions"], daily_sources=small["daily_sources"],
        bars=dict(file="bars.ndjson", sha256=fixture_digest(bars), size_bytes=len(bars), row_count=33),
        normalization=dict(file="normalization.json", sha256=fixture_digest(proof), size_bytes=len(proof), row_count=6),
        policy_sha256=H, materializer_version="daily-price-only-v1", code_commit="1" * 40)
    manifest["content_sha256"] = fixture_digest(encode(manifest))
    manifest_bytes = encode(manifest)
    admission = dict(schema="owner-beta-daily-etf11-admission-v1", content_sha256=manifest["content_sha256"],
        manifest_sha256=fixture_digest(manifest_bytes), policy_sha256=H, parent_admission_sha256=None, base=base,
        verifier_code_commit=verifier * 40, normalizer_version=partial["normalizer_version"],
        evidence_sha256=partial["evidence_sha256"], counts=partial["counts"])
    admission_bytes = encode(admission)
    key = fixture_digest(admission_bytes)
    prefix = "daily/admitted/" + key[7:]
    values.update({prefix + "/admission.json": admission_bytes, prefix + "/manifest.json": manifest_bytes,
                   prefix + "/bars.ndjson": bars, prefix + "/normalization.json": proof})
    session = manifest["daily_sources"][0]
    files = session["bars"] + session["reference"] + [x["source_file"] for x in session["calendar"]]
    files += [p for action in session["actions"] for p in action["pages"]]
    observation = b"owner-beta-daily-source-observation-v1\n[" + b",".join(sorted(set(encode(f) for f in files))) + b"]"
    frontier_sha = fixture_digest(observation)
    frontier = dict(schema="owner-beta-daily-source-frontier-v1", owner=B, policy_sha256=H, admission_sha256=key,
        parent_frontier_sha256=None, range_start="2026-08-29", range_end="2026-08-31",
        observation_sha256=frontier_sha, source_file_count=30)
    directory = "daily/frontiers/" + frontier_sha[7:]
    values.update({directory + "/frontier.json": encode(frontier), directory + "/observation.json": observation})
    transport = dict(admission_sha256=key, source_frontier_sha256=frontier_sha, admission=admission, manifest=manifest)
    row = poll_receipt(result, source_frontier_sha256=frontier_sha,
        admission_sha256=key if result == "VERIFIED" else None,
        completion_binding_sha256=fixture_digest(b"sql-admission-v1" + fixture_jsonb(transport)))
    db = db_row("owner_beta_daily_admissions", admission_sha256=key, owner_user_id=B, policy_sha256=H,
        content_sha256=manifest["content_sha256"], manifest_sha256=admission["manifest_sha256"], parent_admission_sha256=None,
        range_start="2016-08-29", range_end="2026-08-31", session_count=3, bar_count=33,
        evidence_json=transport, admitted_at="2026-11-06T07:50:00.000000Z")
    calendar_file = session["calendar"][0]["source_file"]
    rows = fixture_calendar_projection(calendar_file, values["raw/provider=kis/market=kr/date=2026-08-31/batch=" + batch_id + "/calendar.json"])
    if result == "VERIFIED": rows["owner_beta_daily_admissions"] = [(key, db)]
    if result != "VERIFIED":
        event = source_event(frontier_sha, lo="2026-08-31", hi="2026-08-31")
        prior = source_event(J, kind="ACTION", code="NONCASH_ACTION", lo="2026-08-29", hi="2026-08-31")
        prior["recorded_at"] = "2026-11-06T07:59:00.000000Z"
        row["event_sha256"] = event["event_sha256"]
        rows["owner_beta_daily_source_events"] = sorted((e["event_sha256"], e) for e in (event, prior))
    return dict(values=values, raw=raw, row=row, rows=rows, key=key, transport=transport, db=db,
                frontier=frontier, frontier_sha=frontier_sha, prefix=prefix)


def check_admission_poll(fixture):
    references = poll_references([fixture["row"]], **fixture["rows"])
    protected = _PollFixtureBytes(fixture["values"])
    originals = ref._daily_admissions(protected, references, fixture["raw"])
    frontiers = ref._frontiers(protected, references, fixture["raw"], originals)
    ref._calendar_sources(protected, references, fixture["raw"])
    ref._daily_events_receipts(references, frontiers, protected=protected, raw=fixture["raw"], originals=originals)
    return references, protected, originals, frontiers


def fixture_admission_event(admission, kind, evidence, at="2026-11-06T07:59:00.000000Z"):
    identity = fixture_digest(("owner-beta-daily-admission-event-v1\n" + kind + "\n" + admission + "\n" + evidence + "\n").encode())
    return db_row("owner_beta_daily_admission_events", event_sha256=identity, admission_sha256=admission,
        owner_user_id=B, kind=kind, evidence_sha256=evidence, created_at=at)


class PollHistoryTests(unittest.TestCase):
    def test_sealed_calendar_changed_metadata_has_no_fabricated_prior_original(self):
        fixture = daily_calendar_fixture()
        rows = calendar_rows(fixture)
        # Same parsed batch and provider bytes, but replaced batch.json bytes.
        # SQL keeps the old evidence JSON; the Raw store has only this one path.
        changed_batch = fixture["batch_bytes"] + b" "
        file = copy.deepcopy(fixture["file"])
        file["batch_json_sha256"] = fixture_digest(changed_batch)
        raw = ref._RawIndex(ref._Budget(limits()))
        raw.add(fixture["batch"], fixture["prefix"], fixture_digest(changed_batch), fixture_digest(fixture["line"][:-1]), 1)
        raw.close()
        wire = json.dumps(file, separators=(",", ":")).encode()
        observation = fixture_digest(b"owner-beta-daily-source-observation-v1\n[" + wire + b"]")
        identity = fixture_digest(("owner-beta-daily-calendar-source-v1\n" + B + "\n" + H + "\ndaily-raw-reconstruction-v1\n").encode() + wire)
        days = [{k: d[k] for k in ("session_date", "session_type", "evidence_kind", "source_row_ordinal", "source_row_sha256")}
                for _, d in rows["owner_beta_daily_calendar_days"]]
        event = source_event(observation, "CALENDAR", "SEALED_SOURCE_CHANGED", hi="2026-11-07")
        rows["owner_beta_daily_source_events"] = [(event["event_sha256"], event)]
        row = poll_receipt("INPUT_BLOCKED", source_frontier_sha256=observation, event_sha256=event["event_sha256"],
            completion_binding_sha256=fixture_digest(b"sql-calendar-v1" + fixture_jsonb([identity, file, days])))
        values = _PollFixtureBytes({fixture["prefix"] + "/calendar.json": fixture["body"], fixture["prefix"] + "/batch.json": changed_batch})
        self.assertIn(observation, ref._poll_calendar_originals(values, raw, {observation}))
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_DAILY_ORIGINAL$"):
            ref._daily_events_receipts(poll_references([row], **rows), {}, protected=values, raw=raw)

    def test_sql_calendar_timestamp_projection_keeps_exact_original_nanoseconds(self):
        fixture = daily_calendar_fixture()
        fixture["batch"]["retrieved_at"] = "2026-11-06T07:30:00.123456789Z"
        fixture["batch_bytes"] = json.dumps(fixture["batch"], indent=2).encode() + b"\n"
        fixture["line"] = json.dumps(fixture["batch"], separators=(",", ":")).encode() + b"\n"
        fixture["file"].update(batch_json_sha256=fixture_digest(fixture["batch_bytes"]),
            manifest_line_sha256=fixture_digest(fixture["line"][:-1]), retrieved_at="2026-11-06T07:30:00.123456789Z",
            original_retrieved_at="2026-11-06T07:30:00.123456789Z")
        raw = ref._RawIndex(ref._Budget(limits()))
        raw.add(fixture["batch"], fixture["prefix"], fixture_digest(fixture["batch_bytes"]), fixture_digest(fixture["line"][:-1]), 1)
        raw.close()
        rows = fixture_calendar_projection(fixture["file"], fixture["body"])
        source = rows["owner_beta_daily_calendar_sources"][0][1]
        source["source_retrieved_at"] = "2026-11-06T07:30:00.123457Z"
        ref._calendar_sources(_PollFixtureBytes({fixture["prefix"] + "/calendar.json": fixture["body"]}), poll_references([], **rows), raw)
        self.assertEqual(source["evidence_json"]["retrieved_at"], "2026-11-06T07:30:00.123456789Z")
        source["source_retrieved_at"] = "2026-11-06T07:30:00.123456Z"
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_CALENDAR_SOURCE$"):
            ref._calendar_sources(_PollFixtureBytes({fixture["prefix"] + "/calendar.json": fixture["body"]}), poll_references([], **rows), raw)
        for source, expected in (("00.000000500", "00.000000000"), ("00.000001500", "00.000002000"),
                                 ("00.999999999", "01.000000000")):
            self.assertEqual(ref._pg_microsecond("2026-11-06T07:30:" + source + "Z"), "2026-11-06T07:30:" + expected + "Z")

    def test_receipt_owner_and_epoch_are_required_without_current_authority(self):
        for change in ({"owner_user_id": None}, {"activation_epoch": -1}, {"activation_epoch": 2**63}, {"policy_sha256": None}):
            with self.subTest(change=list(change)), self.assertRaises((ref.ReferenceError, ref.audit.AuditError)):
                ref._daily_events_receipts(poll_references([poll_receipt("IO_FAILED", **change)]), {})

    def test_copied_calendar_publication_columns_bind_terminal_original(self):
        values, manifest, _, raw = copied_calendar_fixture(hops=2)
        file = manifest["daily_sources"][0]["calendar"][0]["source_file"]
        body = values["raw/provider=kis/market=kr/date=2026-08-31/batch=" + A + "/calendar.json"]
        rows = fixture_calendar_projection(file, body)
        references = poll_references([], **rows)
        ref._calendar_sources(_PollFixtureBytes(values), references, raw)
        rows["owner_beta_daily_calendar_sources"][0][1]["source_batch_id"] = file["batch_id"]
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_CALENDAR_SOURCE$"):
            ref._calendar_sources(_PollFixtureBytes(values), poll_references([], **rows), raw)

    def test_calendar_proposal_budget_and_copy_rights_are_not_bypassed(self):
        for change in ("budget", "rights"):
            fixture = calendar_poll_fixture(copies=1)
            if change == "budget":
                fixture["raw"].budget = ref._Budget(limits())
                fixture["raw"].budget.used_entries = fixture["raw"].budget.entries
            else:
                terminal = fixture["file"]["copied_from"][-1]
                fixture["raw"].files[tuple(terminal[k] for k in ref._KEY)]["entitlement_reference"] = "foreign-fixture-rights"
            with self.subTest(change=change), self.assertRaises(ref.ReferenceError):
                ref._daily_events_receipts(poll_references([fixture["row"]], **fixture["rows"]), {},
                    protected=_PollFixtureBytes(fixture["values"]), raw=fixture["raw"])

    def test_partial_stream_never_promotes_valid_poll_proposal(self):
        fixture = calendar_poll_fixture()
        rows = {**fixture["rows"], "owner_beta_daily_poll_receipts": [(A, fixture["row"])]}
        stream = rows_stream(rows)
        for cut in (stream[:-1], stream[:-8], stream.split(b'["references_end"]')[0]):
            references = ref._References(limits(), DOC)
            with self.assertRaises(ref.audit.AuditError):
                references.decode(io.BytesIO(cut))
            self.assertTrue(references.failed)
            self.assertEqual(references.tables, {})
            with self.assertRaises(ref.ReferenceError):
                ref._daily_events_receipts(references, {}, protected=_PollFixtureBytes(fixture["values"]), raw=fixture["raw"])

    def test_admission_sealed_changed_keeps_both_original_proposals(self):
        prior = admission_poll_fixture()
        proposed = admission_poll_fixture(result="INPUT_BLOCKED", batch_id=C, verifier="3")
        for path, value in prior["values"].items():
            if path.endswith("manifest.jsonl"):
                proposed["values"][path] = value + proposed["values"][path]
            else:
                self.assertNotIn(path, proposed["values"])
                proposed["values"][path] = value
        proposed["raw"].files.update(prior["raw"].files)
        proposed["raw"].batches.update(prior["raw"].batches)
        proposed["rows"]["owner_beta_daily_admissions"] = [(prior["key"], prior["db"])]
        for table in ("owner_beta_daily_calendar_sources", "owner_beta_daily_calendar_days"):
            proposed["rows"][table] = sorted(proposed["rows"][table] + prior["rows"][table])
        event = source_event(proposed["frontier_sha"], code="SEALED_SOURCE_CHANGED", lo="2026-08-31", hi="2026-08-31")
        event["admission_sha256"] = prior["key"]
        proposed["row"]["event_sha256"] = event["event_sha256"]
        proposed["rows"]["owner_beta_daily_source_events"] = [(event["event_sha256"], event)]
        consequence = fixture_admission_event(prior["key"], "CONFLICT", event["event_sha256"], at=event["recorded_at"])
        proposed["rows"]["owner_beta_daily_admission_events"] = [(consequence["event_sha256"], consequence)]
        _, _, originals, frontiers = check_admission_poll(proposed)
        self.assertEqual(set(originals), {prior["key"], proposed["key"]})
        self.assertEqual(set(frontiers), {prior["frontier_sha"], proposed["frontier_sha"]})
        proposed["rows"].pop("owner_beta_daily_admissions")
        with self.assertRaises(ref.ReferenceError):
            check_admission_poll(proposed)

    def test_existing_admission_revocation_explains_block_without_borrowing_new_event(self):
        fixture = admission_poll_fixture(result="INPUT_BLOCKED")
        fixture["rows"]["owner_beta_daily_admissions"] = [(fixture["key"], fixture["db"])]
        event = next(e for _, e in fixture["rows"]["owner_beta_daily_source_events"] if e["source_kind"] == "BAR")
        event["admission_sha256"] = fixture["key"]
        fixture["rows"]["owner_beta_daily_source_events"] = [(event["event_sha256"], event)]
        revocation = fixture_admission_event(fixture["key"], "REVOKED", J)
        consequence = fixture_admission_event(fixture["key"], "CONFLICT", event["event_sha256"], at=event["recorded_at"])
        fixture["rows"]["owner_beta_daily_admission_events"] = sorted((e["event_sha256"], e) for e in (revocation, consequence))
        check_admission_poll(fixture)
        # The self-created CONFLICT is retained, but cannot explain its own cause.
        fixture["rows"]["owner_beta_daily_admission_events"] = [(consequence["event_sha256"], consequence)]
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_POLL_ADMISSION_CAUSE$"):
            check_admission_poll(fixture)

    def test_later_revocation_preserves_old_success_but_prior_revocation_rejects(self):
        fixture = admission_poll_fixture()
        revoke = fixture_admission_event(fixture["key"], "REVOKED", J, at="2026-11-07T00:00:00.000000Z")
        fixture["rows"]["owner_beta_daily_admission_events"] = [(revoke["event_sha256"], revoke)]
        check_admission_poll(fixture)
        revoke["created_at"] = "2026-11-06T07:59:00.000000Z"
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_POLL_ADMISSION_CAUSE$"):
            check_admission_poll(fixture)

    def test_admission_requires_all_original_calendar_extension_facts(self):
        for change in ("missing", "policy", "future", "closed_stop"):
            fixture = admission_poll_fixture()
            if change == "missing": fixture["rows"]["owner_beta_daily_calendar_days"].pop(0)
            elif change == "policy": fixture["rows"]["owner_beta_daily_calendar_sources"][0][1]["policy_sha256"] = J
            elif change == "future": fixture["rows"]["owner_beta_daily_calendar_sources"][0][1]["projected_at"] = "2026-11-07T00:00:00.000000Z"
            else: fixture["rows"]["owner_beta_daily_calendar_days"][-1][1]["session_type"] = "CLOSED"
            with self.subTest(change=change), self.assertRaises(ref.ReferenceError):
                check_admission_poll(fixture)

    def test_latest_state_missing_stale_or_misbound_rejects(self):
        row = poll_receipt("INPUT_PENDING", source_frontier_sha256=H)
        references = poll_references([row])
        good = dict(next(iter(references.rows("owner_beta_daily_poll_state"))))
        for change in ("missing", "latest", "time", "result", "admitted", "frontier", "due", "null_latest"):
            state = copy.deepcopy(good)
            if change == "latest": state["latest_receipt_id"] = C
            elif change == "time": state["last_completed_at"] = row["started_at"]
            elif change == "result": state["last_result_code"] = "IO_FAILED"
            elif change == "admitted": state["verified_admission_sha256"] = J
            elif change == "frontier": state["source_frontier_sha256"] = J
            elif change == "due": state["next_poll_at"] = row["started_at"]
            elif change == "null_latest": state.update(latest_receipt_id=None, last_completed_at=None, last_result_code=None, source_frontier_sha256=None)
            rows = [] if change == "missing" else [("ADMISSION", state)]
            with self.subTest(change=change), self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_POLL_STATE$"):
                ref._daily_events_receipts(poll_references([row], owner_beta_daily_poll_state=rows), {})
        newer = poll_receipt(receipt_id=C, started_at="2026-11-07T08:00:00.000000Z")
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_POLL_STATE$"):
            ref._daily_events_receipts(poll_references([row, newer], owner_beta_daily_poll_state=[("ADMISSION", good)]), {})

    def test_schedule_pins_nulls_and_completion_timestamp_shapes(self):
        rows = []
        for key in ("source_frontier_sha256", "admission_sha256", "event_sha256", "completion_binding_sha256"):
            rows.append(poll_receipt("CHECKED", "SCHEDULE", **{key: H}))
            rows.append(poll_receipt(**{key: H}))
        rows += [poll_receipt("INPUT_PENDING", completed_at=None), poll_receipt(completed_at="2026-11-06T08:00:01.000000Z"),
                 poll_receipt("IO_FAILED", completed_at="2026-11-06T07:00:00.000000Z"), poll_receipt("UNKNOWN")]
        for i, row in enumerate(rows):
            with self.subTest(case=i), self.assertRaises(ref.ReferenceError):
                ref._daily_events_receipts(poll_references([row]), {})

    def test_explicit_block_exact_independent_jsonb_vector_and_all_codes(self):
        event = source_event()
        expected = ('{"code": "SOURCE_CONFLICT", "range_end": "2026-11-06", "range_start": "2026-11-06", '
                    '"source_kind": "BAR", "evidence_sha256": "' + H + '"}').encode()
        payload = {k: event[k] for k in ("source_kind", "range_start", "range_end", "code", "evidence_sha256")}
        self.assertEqual(fixture_jsonb(payload), expected)
        self.assertEqual(ref._pg_jsonb(payload), expected)
        for kind in ("ACTION", "CALENDAR", "BAR", "REFERENCE"):
            for code in ("SOURCE_CONFLICT", "SEALED_SOURCE_CHANGED", "SOURCE_INTEGRITY", "CALENDAR_CONFLICT", "NONCASH_ACTION"):
                event = source_event(kind=kind, code=code)
                row = block_receipt(event)
                ref._daily_events_receipts(poll_references([row],
                    owner_beta_daily_source_events=[(event["event_sha256"], event)]), {})

    def test_explicit_block_missing_foreign_future_and_replaced_event(self):
        for change in ("missing", "owner", "future", "replaced", "revoked", "admission", "no_binding"):
            event = source_event(code="SOURCE_REVOKED" if change == "revoked" else "SOURCE_CONFLICT")
            row = block_receipt(event)
            if change == "owner": row["owner_user_id"] = C
            elif change == "future": event["recorded_at"] = "2026-11-07T08:00:00.000000Z"
            elif change == "replaced": event = source_event(J)
            elif change == "admission": row["admission_sha256"] = H
            elif change == "no_binding": row["completion_binding_sha256"] = None
            rows = [] if change == "missing" else [(event["event_sha256"], event)]
            with self.subTest(change=change), self.assertRaises(ref.ReferenceError):
                ref._daily_events_receipts(poll_references([row], owner_beta_daily_source_events=rows), {})

    def test_calendar_unpublished_proposal_has_original_binding_sorted_days_and_copy_chain(self):
        for copies in (0, 1, 2):
            fixture = calendar_poll_fixture(copies=copies, policy=J)
            refs = poll_references([fixture["row"]], **fixture["rows"])
            self.assertNotIn(fixture["identity"], refs.keyed("owner_beta_daily_calendar_sources", "calendar_evidence_sha256"))
            ref._daily_events_receipts(refs, {}, protected=_PollFixtureBytes(fixture["values"]), raw=fixture["raw"])
            self.assertEqual([d["source_row_ordinal"] for d in fixture["days"]], [1, 0])
            self.assertEqual(ref._retained_file_evidence(fixture["raw"],
                tuple(fixture["file"][k] for k in ref._KEY)), fixture["file"])

    def test_calendar_changed_binding_body_or_original_and_deleted_cause(self):
        for change in ("binding", "body", "missing", "cause", "future", "after_event", "frontier", "policy", "no_conflict", "day_order"):
            fixture = calendar_poll_fixture(conflict=change != "no_conflict")
            if change == "binding": fixture["row"]["completion_binding_sha256"] = J
            elif change == "body": fixture["values"][fixture["path"]] += b" "
            elif change == "missing": fixture["raw"].files.pop(tuple(fixture["file"][k] for k in ref._KEY))
            elif change == "cause":
                fixture["rows"].pop("owner_beta_daily_calendar_sources")
                fixture["rows"].pop("owner_beta_daily_calendar_days")
            elif change == "future": fixture["rows"]["owner_beta_daily_calendar_sources"][0][1]["projected_at"] = "2026-11-07T00:00:00.000000Z"
            elif change == "after_event": fixture["rows"]["owner_beta_daily_calendar_sources"][0][1]["projected_at"] = "2026-11-06T08:00:00.750000Z"
            elif change == "frontier": fixture["row"]["source_frontier_sha256"] = H
            elif change == "policy": fixture["row"]["policy_sha256"] = J
            elif change == "day_order":
                fixture["row"]["completion_binding_sha256"] = fixture_digest(b"sql-calendar-v1" + fixture_jsonb([
                    fixture["identity"], fixture["file"], list(reversed(fixture["days"]))]))
            with self.subTest(change=change), self.assertRaises(ref.ReferenceError):
                ref._daily_events_receipts(poll_references([fixture["row"]], **fixture["rows"]), {},
                    protected=_PollFixtureBytes(fixture["values"]), raw=fixture["raw"])

    def test_calendar_sealed_changed_does_not_invent_replaced_original(self):
        fixture = calendar_poll_fixture(conflict=False)
        event = source_event(fixture["row"]["source_frontier_sha256"], "CALENDAR", "SEALED_SOURCE_CHANGED", hi="2026-11-07")
        fixture["row"]["event_sha256"] = event["event_sha256"]
        fixture["rows"]["owner_beta_daily_source_events"] = [(event["event_sha256"], event)]
        # Distinct retained batch IDs cannot manufacture a same-identity change.
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_POLL_CALENDAR_CAUSE$"):
            ref._daily_events_receipts(poll_references([fixture["row"]], **fixture["rows"]), {},
                protected=_PollFixtureBytes(fixture["values"]), raw=fixture["raw"])

    def test_admission_verified_and_unpublished_conflict_reopen_retained_originals(self):
        for result in ("VERIFIED", "INPUT_BLOCKED"):
            fixture = admission_poll_fixture(result=result)
            refs, protected, originals, frontiers = check_admission_poll(fixture)
            self.assertEqual(fixture["key"] in refs.keyed("owner_beta_daily_admissions", "admission_sha256"), result == "VERIFIED")
            self.assertIn(fixture["key"], originals)
            self.assertIn(fixture["frontier_sha"], frontiers)
            with self.assertRaises(ref.ReferenceError):
                ref.describe(protected, io.BytesIO(rows_stream()))

    def test_admission_rebound_binding_missing_proposal_frontier_owner_and_cause(self):
        for change in ("binding", "admission_bytes", "manifest_bytes", "missing_frontier", "owner", "policy", "cause", "future_cause", "event_range"):
            fixture = admission_poll_fixture(result="INPUT_BLOCKED")
            if change == "binding": fixture["row"]["completion_binding_sha256"] = J
            elif change == "admission_bytes": fixture["values"][fixture["prefix"] + "/admission.json"] += b" "
            elif change == "manifest_bytes": fixture["values"][fixture["prefix"] + "/manifest.json"] += b" "
            elif change == "missing_frontier":
                fixture["values"] = {p: v for p, v in fixture["values"].items() if not p.startswith("daily/frontiers/")}
            elif change == "owner": fixture["row"]["owner_user_id"] = C
            elif change == "policy": fixture["row"]["policy_sha256"] = J
            elif change in ("cause", "future_cause"):
                rows = fixture["rows"]["owner_beta_daily_source_events"]
                if change == "cause": fixture["rows"]["owner_beta_daily_source_events"] = [(k, e) for k, e in rows if e["source_kind"] == "BAR"]
                else:
                    next(e for _, e in rows if e["source_kind"] == "ACTION")["recorded_at"] = "2026-11-07T00:00:00.000000Z"
            else:
                event = source_event(fixture["frontier_sha"], lo="2026-08-30", hi="2026-08-31")
                fixture["row"]["event_sha256"] = event["event_sha256"]
                fixture["rows"]["owner_beta_daily_source_events"] = sorted([(event["event_sha256"], event),
                    *[(k, e) for k, e in fixture["rows"]["owner_beta_daily_source_events"] if e["source_kind"] == "ACTION"]])
            with self.subTest(change=change), self.assertRaises(ref.ReferenceError):
                check_admission_poll(fixture)

    def test_retained_admission_mutation_after_provisional_stream(self):
        fixture = admission_poll_fixture()
        refs, protected, originals, frontiers = check_admission_poll(fixture)
        fixture["values"][fixture["prefix"] + "/admission.json"] += b"\n"
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_POLL_ORIGINAL$"):
            ref._daily_events_receipts(refs, frontiers, protected=protected, raw=fixture["raw"], originals=originals)

    def test_all_finish_results_and_diagnostic_hashes_are_archival(self):
        for stage, results in (("SCHEDULE", ("CHECKED", "DEFERRED", "BLOCKED")),
                               ("ADMISSION", ("INPUT_PENDING", "INPUT_BLOCKED", "IO_FAILED"))):
            for result in results:
                for frontier in ((None,) if stage == "SCHEDULE" else (None, H)):
                    row = poll_receipt(result, stage, source_frontier_sha256=frontier)
                    with self.subTest(stage=stage, result=result, diagnostic=frontier is not None):
                        ref._daily_events_receipts(poll_references([row]), {})

    def test_superseded_unfinished_and_historical_policy_owner_epoch(self):
        old = poll_receipt(receipt_id=A, owner_user_id=C, policy_sha256=J, activation_epoch=999)
        current = poll_receipt("INPUT_PENDING", receipt_id=D, started_at="2026-11-07T08:00:00.000000Z",
            completed_at="2026-11-07T08:00:01.000000Z", activation_epoch=1)
        ref._daily_events_receipts(poll_references([old, current]), {})
        # No current enable/rights/cohort row is needed to retain either history.
        current.update(completed_at=None, result_code=None)
        ref._daily_events_receipts(poll_references([old, current]), {})

    def test_stage_result_and_unfinished_shapes_reject_with_rebound_stream(self):
        for row in (poll_receipt("CHECKED"), poll_receipt("VERIFIED", "SCHEDULE"),
                    poll_receipt("IO_FAILED", "unknown"), poll_receipt(source_frontier_sha256=H),
                    poll_receipt("INPUT_PENDING", completion_binding_sha256=H)):
            with self.subTest(stage=row["stage"], result=row["result_code"]), self.assertRaises(ref.ReferenceError):
                ref._daily_events_receipts(poll_references([row]), {})

    def test_explicit_block_cannot_ignore_binding_or_borrow_evidence(self):
        event = source_event()
        for frontier in (H, J):
            row = poll_receipt("INPUT_BLOCKED", source_frontier_sha256=frontier,
                event_sha256=event["event_sha256"], completion_binding_sha256=J)
            with self.subTest(frontier=frontier), self.assertRaises(ref.ReferenceError):
                ref._daily_events_receipts(poll_references([row],
                    owner_beta_daily_source_events=[(event["event_sha256"], event)]), {})


class DailyOriginalTests(unittest.TestCase):
    def test_unfinished_latest_poll_cannot_retain_prior_completion(self):
        receipt = db_row("owner_beta_daily_poll_receipts", receipt_id=A, stage="ADMISSION", owner_user_id=B,
            policy_sha256=H, activation_epoch=1, started_at="2026-11-06T07:30:00.000000Z")
        state = db_row("owner_beta_daily_poll_state", stage="ADMISSION", latest_receipt_id=A,
            next_poll_at="2026-11-06T07:31:00.000000Z")
        def check(value):
            references = ref._References(limits(), DOC)
            references.decode(io.BytesIO(rows_stream({"owner_beta_daily_poll_receipts": [(A, receipt)],
                                                      "owner_beta_daily_poll_state": [("ADMISSION", value)]})))
            ref._daily_events_receipts(references, {})
        # daily_0060_take_slot clears all four completion fields atomically.
        check(state)
        prior = {"last_completed_at": "2026-11-05T07:31:00.000000Z", "last_result_code": "VERIFIED",
                 "verified_admission_sha256": H, "source_frontier_sha256": J}
        for field in prior:
            with self.subTest(field=field), self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_POLL_STATE$"):
                check({**state, field: prior[field]})
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_POLL_STATE$"):
            check({**state, **prior})

    def test_verified_poll_cannot_omit_admission_and_frontier(self):
        row = db_row("owner_beta_daily_poll_receipts", receipt_id=A, stage="ADMISSION", owner_user_id=B,
            policy_sha256=H, activation_epoch=1, started_at="2026-11-06T07:30:00.000000Z",
            completed_at="2026-11-06T07:31:00.000000Z", result_code="VERIFIED")
        references = ref._References(limits(), DOC)
        references.decode(io.BytesIO(rows_stream({"owner_beta_daily_poll_receipts": [(A, row)]})))
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_POLL_ADMISSION$"):
            ref._daily_events_receipts(references, {})

    def test_completion_jsonb_wire_is_distinct_from_archive_canonical(self):
        value = {"long": Decimal("1.00"), "z": True, "a": None, "한": "é"}
        self.assertEqual(ref._pg_jsonb(value), '{"a": null, "z": true, "한": "é", "long": 1.00}'.encode())
        self.assertNotEqual(ref._pg_jsonb(value), ref.audit.canonical(value))
        with self.assertRaises(ref.ReferenceError):
            ref._pg_jsonb(Decimal("1e999999999"))

    def test_nested_proof_links_actual_raw_calendar_and_all_document_sources(self):
        originals, manifest, admission, index = nested_daily_fixture()
        ref._daily_nested(_FixtureBytes(originals), "daily/admitted/" + H[7:], manifest, admission, index)

    def test_nested_proof_mutation_missing_calendar_document_and_prior_frontier(self):
        for change in ("proof", "calendar", "document", "frontier", "count", "pin"):
            originals, manifest, admission, index = nested_daily_fixture()
            session = manifest["daily_sources"][0]
            if change == "proof": originals["daily/admitted/" + H[7:] + "/normalization.json"] = originals["daily/admitted/" + H[7:] + "/normalization.json"].replace(b'"documents":', b'"unexpected":')
            elif change == "calendar": session["calendar"].pop(0)
            elif change == "document": session["normalized_documents"].pop()
            elif change == "frontier": session["previous_frontier"] = "2026-08-27"
            elif change == "count": admission["counts"]["source_file_count"] -= 1
            else: session["bars"][0]["manifest_line_sha256"] = J
            with self.subTest(change=change), self.assertRaises(ref.ReferenceError):
                ref._daily_nested(_FixtureBytes(originals), "daily/admitted/" + H[7:], manifest, admission, index)
    def test_real_decoder_calendar_reference_and_original_closure(self):
        fixture = daily_calendar_fixture()
        references = ref._References(limits(), DOC)
        references.decode(io.BytesIO(rows_stream(calendar_rows(fixture))))
        originals = _FixtureBytes({fixture["prefix"] + "/calendar.json": fixture["body"]})
        result = ref._calendar_sources(originals, references, fixture["index"])
        self.assertEqual(len(result), 1)
        with self.assertRaises(ref.ReferenceError):
            ref.describe(originals, io.BytesIO(rows_stream()))

    def test_calendar_db_deleted_changed_cross_owner_and_mixed_pin(self):
        for change in ("deleted", "changed", "owner", "pin", "body"):
            fixture = daily_calendar_fixture()
            rows = calendar_rows(fixture)
            if change == "deleted": rows["owner_beta_daily_calendar_days"].pop()
            elif change == "changed": rows["owner_beta_daily_calendar_days"][1][1]["session_type"] = "TRADING"
            elif change == "owner": rows["owner_beta_daily_calendar_days"][1][1]["owner_user_id"] = C
            elif change == "pin": rows["owner_beta_daily_calendar_sources"][0][1]["source_manifest_line_sha256"] = J
            else: fixture["body"] = fixture["body"].replace(b'"N"', b'"Y"')
            references = ref._References(limits(), DOC)
            # Cross-owner wire keys must change too; equality remains a decoder prerequisite.
            if change == "owner":
                key, row = rows["owner_beta_daily_calendar_days"].pop()
                rows["owner_beta_daily_calendar_days"].append((key.replace(B, C), row))
            references.decode(io.BytesIO(rows_stream(rows)))
            originals = _FixtureBytes({fixture["prefix"] + "/calendar.json": fixture["body"]})
            with self.subTest(change=change), self.assertRaises(ref.ReferenceError):
                ref._calendar_sources(originals, references, fixture["index"])

    def test_adopted_legacy_registry_exact_original_and_mutations(self):
        path = PATH.parents[3] / "configs/evidence/kis-historical-price-only-beta-approved-artifacts.json"
        data = path.read_bytes()
        approved = ref._legacy_approval(data)
        self.assertEqual(approved["session_count"], 1608)
        self.assertEqual(approved["bar_count"], 17688)
        for changed in (data + b"\n", data.replace(b'"session_count":1608', b'"session_count":1609'),
                        data.replace(b'PRICE_RETURN_ONLY', b'TOTAL_RETURN_ONLY')):
            with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_LEGACY_APPROVAL$"):
                ref._legacy_approval(changed)

    def test_equivalent_raw_duplicates_retain_every_line_and_first_pin(self):
        fixture = daily_calendar_fixture()
        batch = copy.deepcopy(fixture["batch"])
        batch["retrieved_at"] = "2026-11-06T16:30:00+09:00"
        batch["files"][0].update(response_continuation=None, copied_from=None)
        second = json.dumps(batch, indent=1).encode() + b"\n"
        second_hash = "sha256:" + hashlib.sha256(second[:-1]).hexdigest()
        fixture["index"].add(batch, fixture["prefix"], fixture["file"]["batch_json_sha256"], second_hash, 2)
        original = fixture["index"].reference(fixture["file"])
        self.assertEqual(original["manifest_lines"], [[fixture["file"]["manifest_line_sha256"], 1], [second_hash, 2]])
        self.assertEqual(original["line_sha256"], fixture["file"]["manifest_line_sha256"])
        self.assertEqual(ref._file_evidence(fixture["file"], fixture["index"]), fixture["file"])

    def test_duplicate_conflicts_do_not_disappear(self):
        for change in ("rights", "retrieval", "date", "kind", "size", "content", "request", "marker"):
            fixture = daily_calendar_fixture()
            batch = copy.deepcopy(fixture["batch"])
            item = batch["files"][0]
            if change == "rights": batch["entitlement_reference"] = "different"
            elif change == "retrieval": batch["retrieved_at"] = "2026-11-06T07:30:01Z"
            elif change == "date": batch["date"] = "2026-11-07"
            elif change == "kind": item["kind"] = "bars"
            elif change == "size": item["size_bytes"] += 1
            elif change == "content": item["content_hash"] = H
            elif change == "request": item["request"]["query"].reverse()
            else: item["response_continuation"] = "M"
            with self.subTest(change=change), self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_RAW_DUPLICATE_CONFLICT$"):
                fixture["index"].add(batch, fixture["prefix"], fixture["file"]["batch_json_sha256"], H, 2)

    def test_u64_raw_size_does_not_accept_bool_or_float_spelling(self):
        for number in (b"true", b"1.0", b"1e0", b"18446744073709551616"):
            batch = raw_batch(body=b"x")
            wire = json.dumps(batch, separators=(",", ":")).encode().replace(b'"size_bytes":1', b'"size_bytes":' + number)
            with self.subTest(number=number), self.assertRaises(ref.ReferenceError):
                index_batch(ref._RawIndex(ref._Budget(limits())), ref._raw_json(wire))

    def test_daily_original_exact_pin_mutations_and_no_freeform_errors(self):
        for field in ("wire_sha256", "batch_json_sha256", "manifest_line_sha256", "original_retrieved_at", "kind", "batch_date"):
            fixture = daily_calendar_fixture()
            changed = copy.deepcopy(fixture["file"])
            changed[field] = "secret-provider-prose"
            with self.subTest(field=field), self.assertRaises(ref.ReferenceError) as caught:
                ref._file_evidence(changed, fixture["index"])
            self.assertNotIn("secret-provider-prose", str(caught.exception))

    def test_independent_observation_bytes_and_duplicate_collapse(self):
        fixture = daily_calendar_fixture()
        expected = b"owner-beta-daily-source-observation-v1\n[" + json.dumps(fixture["file"], ensure_ascii=False, separators=(",", ":")).encode() + b"]"
        actual, files = ref._observation([fixture["file"], copy.deepcopy(fixture["file"])], fixture["index"])
        self.assertEqual(actual, expected)
        self.assertEqual(files, [fixture["file"]])
        with self.assertRaises(ref.ReferenceError):
            ref._observation([], fixture["index"])

    def test_actual_calendar_body_independent_row_commitment(self):
        fixture = daily_calendar_fixture()
        days = ref._calendar_provider_days(ref._json(fixture["body"]), "2026-11-06")
        self.assertEqual(days["2026-11-06"], {"session_date": "2026-11-06", "session_type": "TRADING",
            "evidence_kind": "REQUESTED_TARGET", "source_row_ordinal": 0,
            "source_row_sha256": "sha256:" + hashlib.sha256(b'{"bass_dt":"20261106","opnd_yn":"Y"}').hexdigest()})
        self.assertEqual(days["2026-11-07"]["session_type"], "CLOSED")
        self.assertEqual(days["2026-11-07"]["evidence_kind"], "DERIVED_SURROUNDING")

    def test_calendar_duplicate_deleted_target_wrong_day_and_provider_failure(self):
        for change in ("duplicate", "deleted", "date", "status", "provider"):
            value = json.loads(daily_calendar_fixture()["body"])
            if change == "duplicate": value["output"].append(value["output"][0])
            elif change == "deleted": value["output"].pop(0)
            elif change == "date": value["output"][0]["bass_dt"] = "20260230"
            elif change == "status": value["output"][0]["opnd_yn"] = "maybe"
            else: value["rt_cd"] = "1"
            with self.subTest(change=change), self.assertRaises(ref.ReferenceError):
                ref._calendar_provider_days(value, "2026-11-06")

    def test_source_json_does_not_use_archive_decimal_equivalence(self):
        self.assertEqual(ref._source_json(ref._raw_json(b'{"z":1,"a":"1","b":true,"n":null}')),
                         b'{"a":"1","b":true,"n":null,"z":1}')
        for number in ("1.0", "0.01", "1e1"):
            with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_SOURCE_NUMBER_UNIMPLEMENTED$"):
                ref._source_json(ref._raw_json(('{"x":' + number + '}').encode()))

    def test_normalized_source_identity_uses_independently_constructed_struct_order(self):
        fixture = daily_calendar_fixture()
        key = {k: fixture["file"][k] for k in ("provider", "market", "batch_id", "file_name", "wire_sha256")}
        source = {"schema": "owner-beta-daily-normalized-source-v1", "kind": "calendar", "session_date": "2026-11-06", "source_files": [key]}
        digest = "sha256:" + hashlib.sha256(json.dumps(source, separators=(",", ":")).encode()).hexdigest()
        document = {"kind": "calendar", "session_date": "2026-11-06", "normalizer_version": "daily-raw-reconstruction-v1",
                    "sha256": H, "size_bytes": 100, "row_count": 1, "source_files": [key], "source_evidence_sha256": digest}
        identities = {tuple(key.values())}
        self.assertEqual(ref._normalized_document(document, identities), document)
        for change in ("hash", "missing", "duplicate", "date"):
            changed = copy.deepcopy(document)
            keys = identities
            if change == "hash": changed["source_evidence_sha256"] = J
            elif change == "missing": keys = set()
            elif change == "duplicate": changed["source_files"].append(key)
            else: changed["session_date"] = "2026-11-07"
            with self.subTest(change=change), self.assertRaises(ref.ReferenceError):
                ref._normalized_document(changed, keys)


class FramingTests(unittest.TestCase):
    def test_independent_unicode_boolean_null_numeric_frames(self):
        root = ref._FramedRoot()
        row = {"path": "가😀", "flag": True, "null": None, "number": 1, "string": "1"}
        root.add("source_member", "c3a9", row)
        expected = independent_frame("source_member", "c3a9", row)
        result = root._description()
        self.assertEqual(result, {"group": "source_files", "sha256": "sha256:" + hashlib.sha256(expected).hexdigest(),
                                  "entries": 1, "bytes": len(expected)})

    def test_length_prefix_disambiguates_and_byte_length_is_not_characters(self):
        first, second = ref._FramedRoot(), ref._FramedRoot()
        first.add("ab", "c", {"x": "é"})
        second.add("a", "bc", {"x": "é"})
        self.assertNotEqual(first._description()["sha256"], second._description()["sha256"])
        self.assertEqual(first.bytes, len(independent_frame("ab", "c", {"x": "é"})))

    def test_order_duplicate_overflow_and_empty(self):
        root = ref._FramedRoot()
        self.assertEqual(root._description()["sha256"], "sha256:" + hashlib.sha256(b"").hexdigest())
        root.add("x", "b", {})
        for key in ("a", "b"):
            with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_ORDER$"):
                root.add("x", key, {})
        root.bytes = ref.I64
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_OVERFLOW$"):
            root.add("x", "c", {})

    def test_document_duplicate_depth_nonfinite_and_size_rejection(self):
        for raw in (b'{"a":1,"a":2}', b'{"a":NaN}', b'{"a":' + b'[' * 65 + b']' * 65 + b'}', b'{}tail'):
            with self.subTest(raw=raw[:8]), self.assertRaises(ref.ReferenceError):
                ref._json(raw)
        with self.assertRaises(ref.ReferenceError):
            ref._json(b'{}', maximum=1)

    def test_bounded_retained_index_and_types(self):
        budget = ref._Budget(limits(entries=6, manifest=1024))
        for _ in range(6):
            budget.add({"a": 1})
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_INDEX_LIMIT$"):
            budget.add({})
        with self.assertRaises(ref.ReferenceError):
            ref._Budget(limits(manifest=1024)).add({"data": "x" * 1024})
        for value in (True, -1, Decimal("1.5"), ref.I64 + 1):
            with self.assertRaises(ref.ReferenceError):
                ref._integer(value)


class StreamTests(unittest.TestCase):
    def test_all_29_reference_tables_and_nine_groups_are_consumed(self):
        index = ref._References(limits(), DOC)
        result = index.decode(io.BytesIO(rows_stream({"accounts": [(A, account())]})))
        self.assertEqual(len(index.tables), 29)
        self.assertEqual(len(result["groups"]), 9)
        self.assertEqual(index.tables["accounts"][A]["name"], "가😀")
        self.assertNotIn("stopped", result)
        self.assertEqual(result["source_files"], "REQUIRED_EXTERNAL_EVIDENCE")

    def test_every_line_truncation_invalidates_provisional_rows(self):
        raw = rows_stream({"accounts": [(A, account())]})
        lines = raw.splitlines(keepends=True)
        for n in range(len(lines)):
            index = ref._References(limits(), DOC)
            with self.subTest(n=n), self.assertRaises((ref.ReferenceError, ref.audit.AuditError)):
                index.decode(io.BytesIO(b"".join(lines[:n])))
            self.assertTrue(index.failed)
            self.assertEqual(index.tables, {})
            with self.assertRaises(ref.ReferenceError):
                index.decode(io.BytesIO(raw))

    def test_changed_reference_after_database_rows_rejected(self):
        raw = rows_stream({"accounts": [(A, account())]})
        lines = raw.splitlines()
        for n, line in enumerate(lines):
            value = json.loads(line)
            if value[0] == "reference":
                value[3]["initial_cash"] = "2.0000"
                lines[n] = json.dumps(value, separators=(",", ":")).encode()
        index = ref._References(limits(), DOC)
        with self.assertRaisesRegex(ref.audit.AuditError, "^REFERENCE_SET$"):
            index.decode(io.BytesIO(b"\n".join(lines) + b"\n"))
        self.assertTrue(index.failed)
        self.assertEqual(index.tables, {})

    def test_partial_final_line_transport_exception_and_trailing_data(self):
        raw = rows_stream()
        for bad in (raw[:-1], raw + b'[]\n'):
            index = ref._References(limits(), DOC)
            with self.assertRaises(Exception):
                index.decode(io.BytesIO(bad))
            self.assertTrue(index.failed)
        class Broken:
            def readline(self, maximum):
                raise OSError("SECRET PROVIDER BODY")
        index = ref._References(limits(), DOC)
        with self.assertRaises(OSError):
            index.decode(Broken())
        self.assertTrue(index.failed)

    def test_detached_dictionaries_and_snapshots_rejected_without_filesystem_use(self):
        for value in ({}, {"manifest": {}, "verified": True}, object(), None):
            with self.subTest(value=type(value).__name__), self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_SNAPSHOT$"):
                ref.describe(value, io.BytesIO(rows_stream()))

    def test_cancellation_invalidates_all_provisional_rows(self):
        raw = rows_stream({"accounts": [(A, account())]})
        class Cancelled(io.BytesIO):
            def readline(self, maximum=-1):
                line = super().readline(maximum)
                if line == b'["references_end"]\n':
                    raise KeyboardInterrupt()
                return line
        index = ref._References(limits(), DOC)
        with self.assertRaises(KeyboardInterrupt):
            index.decode(Cancelled(raw))
        self.assertTrue(index.failed)
        self.assertEqual(index.tables, {})

    def test_reference_order_and_duplicate_rejected(self):
        raw = rows_stream({"accounts": [(A, account())]})
        line = next(x for x in raw.splitlines(keepends=True) if x.startswith(b'["reference"'))
        index = ref._References(limits(), DOC)
        with self.assertRaises(Exception):
            index.decode(io.BytesIO(raw.replace(line, line + line)))
        self.assertTrue(index.failed)

    def test_job_attempt_rows_are_captured_by_same_decoder(self):
        job = db_row("jobs", id=A)
        attempt = db_row("job_attempts", id=B, job_id=A)
        index = ref._References(limits(), DOC)
        index.decode(io.BytesIO(rows_stream({"jobs": [(A, job)], "job_attempts": [(B, attempt)]})))
        self.assertEqual(index.jobs[A]["id"], A)
        self.assertEqual(index.attempts[B]["job_id"], A)


class RawClosureTests(unittest.TestCase):
    def test_actual_storage_contract_metadata_copy_chain(self):
        original = raw_batch()
        copied = raw_batch(B, copied=copy_reference(original))
        index = ref._RawIndex(ref._Budget(limits()))
        index_batch(index, copied)
        index_batch(index, original)
        index.close()
        identity = {"provider": "kis", "market": "kr", "batch_id": B, "file_name": "bars.json",
                    "wire_sha256": copied["files"][0]["content_hash"]}
        self.assertEqual(index.reference(identity)["file"]["copied_from"], copy_reference(original))

    def test_dangling_changed_request_hash_and_retrieval_fail(self):
        original = raw_batch()
        for mutation in ("missing", "request", "hash", "retrieved_at"):
            index = ref._RawIndex(ref._Budget(limits()))
            copied = raw_batch(B, copied=copy_reference(original))
            if mutation == "request":
                copied["files"][0]["request"]["endpoint"] = "changed"
            elif mutation == "hash":
                copied["files"][0]["copied_from"]["content_hash"] = H
            elif mutation == "retrieved_at":
                copied["files"][0]["copied_from"]["retrieved_at"] = "2026-11-07T00:00:00Z"
            index_batch(index, copied)
            if mutation != "missing":
                index_batch(index, original)
            with self.subTest(mutation=mutation), self.assertRaises(ref.ReferenceError):
                index.close()

    def test_cycle_and_duplicate_rejected(self):
        original = raw_batch()
        copied = raw_batch(B, copied=copy_reference(original))
        original["files"][0]["copied_from"] = copy_reference(copied)
        index = ref._RawIndex(ref._Budget(limits()))
        index_batch(index, original)
        index_batch(index, copied)
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_COPY_CYCLE$"):
            index.close()
        # Identical historical records are admitted by ManifestEntry equality.
        index_batch(index, original)
        changed = copy.deepcopy(original)
        changed["entitlement_reference"] = "changed-rights"
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_RAW_DUPLICATE_CONFLICT$"):
            index_batch(index, changed)

    def test_unknown_fields_duplicate_files_and_unsafe_paths(self):
        for change in ("unknown", "duplicate", "path", "batch"):
            row = raw_batch()
            if change == "unknown":
                row["success"] = True
            elif change == "duplicate":
                row["files"].append(copy.deepcopy(row["files"][0]))
            elif change == "path":
                row["files"][0]["file_name"] = "../password"
            else:
                row["batch_id"] = "../../x"
            with self.subTest(change=change), self.assertRaises(ref.ReferenceError):
                index_batch(ref._RawIndex(ref._Budget(limits())), row)

    def test_lock_secret_and_socket_names_are_forbidden(self):
        for path in ("raw/commit.lock", "paper_config/.env", "config/secret.env", "raw/a.sock", "config/password", "config/key.pem"):
            self.assertTrue(ref._forbidden(path))
        self.assertFalse(ref._forbidden("raw/provider=kis/market=kr/date=2026-11-06/batch=" + A + "/bars.json"))


class SourceLinkTests(unittest.TestCase):
    def fixture(self):
        # A small relationship vector for the actual SQL projections. These
        # private graph helpers do not produce filesystem or serving authority.
        raw = ref._RawIndex(ref._Budget(limits()))
        batch = raw_batch()
        item = batch["files"][0]
        batch["files"] = [{**copy.deepcopy(item), "file_name": "source-%02d.json" % n} for n in range(23)]
        index_batch(raw, batch)
        originals = []
        for item in batch["files"]:
            row = db_row("owner_beta_paper_source_originals", provider="kis", market="kr", batch_id=A,
                         file_name=item["file_name"], wire_sha256=item["content_hash"], slot_id=H)
            key = ":".join(row[k] for k in DOC["archive_identity_key_components"]["owner_beta_paper_source_originals"])
            originals.append((key, row))
        # Independent event commitment algorithm from the source-evidence contract.
        identities = sorted(tuple(row[k] for k in ("provider", "market", "batch_id", "file_name", "wire_sha256"))
                            for _, row in originals)
        original_hash = "sha256:" + hashlib.sha256(("\n".join("|".join(v) for v in identities) + "\n").encode("ascii")).hexdigest()
        table_rows = {
            "owner_beta_paper_source_slots": [(H, db_row("owner_beta_paper_source_slots", slot_id=H,
                owner_user_id=A, policy_sha256=J, session_date="2026-11-06", activation_epoch=3))],
            "owner_beta_paper_source_admissions": [(H, db_row("owner_beta_paper_source_admissions", slot_id=H,
                admission_sha256=H, original_set_sha256=original_hash))],
            "owner_beta_daily_admissions": [(H, db_row("owner_beta_daily_admissions", admission_sha256=H,
                owner_user_id=A, policy_sha256=J, range_end="2026-11-06"))],
            "owner_beta_paper_source_originals": sorted(originals),
            "owner_beta_paper_source_runs": [(H, db_row("owner_beta_paper_source_runs", slot_id=H,
                run_id=B, job_id=C, attempt_id=D))],
            "owner_beta_recommendation_runs": [(B, db_row("owner_beta_recommendation_runs", id=B,
                job_id=C, owner_user_id=A, strategy_config_id=B, as_of="2026-11-06"))],
            "owner_beta_daily_publication_claims": [(B, db_row("owner_beta_daily_publication_claims", run_id=B,
                job_id=C, owner_user_id=A, attempt_id=D, attempt_no=1, claimed_by="fixture-worker"))],
            "jobs": [(C, db_row("jobs", id=C, owner_user_id=A))],
            "job_attempts": [(D, db_row("job_attempts", id=D, job_id=C, attempt_no=1, claimed_by="fixture-worker"))],
        }
        reservation = db_row("owner_beta_daily_reservations", owner_user_id=A, strategy_config_id=B,
            session_date="2026-11-06", trigger_version="owner-beta-daily-trigger-v1", run_id=B, job_id=C,
            admission_sha256=H, activation_epoch=3)
        key = ":".join(str(reservation[k]) for k in DOC["archive_identity_key_components"]["owner_beta_daily_reservations"])
        table_rows["owner_beta_daily_reservations"] = [(key, reservation)]
        refs = ref._References(limits(), DOC)
        refs.decode(io.BytesIO(rows_stream(table_rows)))
        return refs, raw

    def test_slot_admission_original_set_run_job_attempt_closure(self):
        refs, raw = self.fixture()
        ref._source_links(refs, raw)

    def test_dangling_each_chain_link_rejected(self):
        for table in ("owner_beta_paper_source_slots", "owner_beta_paper_source_admissions", "owner_beta_daily_admissions",
                      "owner_beta_recommendation_runs", "owner_beta_daily_reservations", "owner_beta_daily_publication_claims"):
            refs, raw = self.fixture()
            refs.tables[table].clear()
            with self.subTest(table=table), self.assertRaises(ref.ReferenceError):
                ref._source_links(refs, raw)
        for attr in ("jobs", "attempts"):
            refs, raw = self.fixture()
            getattr(refs, attr).clear()
            with self.subTest(attr=attr), self.assertRaises(ref.ReferenceError):
                ref._source_links(refs, raw)

    def test_cross_owner_attempt_and_mixed_epoch_rejected(self):
        cases = (("owner_beta_recommendation_runs", "owner_user_id", B),
                 ("owner_beta_daily_reservations", "activation_epoch", 4),
                 ("owner_beta_daily_publication_claims", "attempt_id", C),
                 ("owner_beta_daily_publication_claims", "claimed_by", "different"),
                 ("owner_beta_paper_source_admissions", "original_set_sha256", J))
        for table, field, value in cases:
            refs, raw = self.fixture()
            next(iter(refs.tables[table].values()))[field] = value
            with self.subTest(table=table, field=field), self.assertRaises(ref.ReferenceError):
                ref._source_links(refs, raw)

    def test_deleted_original_cannot_be_normalized_out(self):
        refs, raw = self.fixture()
        refs.tables["owner_beta_paper_source_originals"].popitem()
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_ORIGINAL_SET$"):
            ref._source_links(refs, raw)


class FixedContractTests(unittest.TestCase):
    def test_checked_in_policy_and_config_originals(self):
        root = Path(__file__).resolve().parents[2]
        data = (root / "tests/fixtures/owner-beta-daily-install/config.json").read_bytes()
        config = ref.daily.validate_config(data, "a" * 40)
        policy = (root / "configs/evidence/owner-beta-daily-price-policy-v1.json").read_bytes()
        ref.daily.validate_policy(policy)
        self.assertEqual(config["policy_sha256"], "sha256:" + hashlib.sha256(policy).hexdigest())
        for changed in (policy + b"\n", policy.replace(b"PRICE_RETURN_ONLY", b"CHANGED_PRICE")):
            with self.assertRaises(Exception):
                ref.daily.validate_policy(changed)

    def test_exact_approved_registry_bytes_and_mutation(self):
        root = Path(__file__).resolve().parents[2]
        registry = (root / "configs/evidence/kis-historical-price-only-v3-approved-artifacts.json").read_bytes()
        record = ref.simulation.approved_record(registry)
        self.assertEqual(record["bars_size_bytes"], 7672914)
        for changed in (registry + b"\n", registry.replace(b"7672914", b"7672915")):
            with self.assertRaises(Exception):
                ref.simulation.approved_record(changed)

    def test_finite_huge_numeric_rejected_before_integer_allocation(self):
        with self.assertRaises(ref.ReferenceError):
            ref._integer(Decimal("1e999999999"))
        with self.assertRaises(ref.ReferenceError):
            ref._integer(Decimal("Infinity"))




class _JournalFixtureBytes:
    """Pure fixture reader, deliberately rejected by prepare_references."""
    def __init__(self, values):
        self.values = values
        self.entries = {}
        for path, raw in values.items():
            parts = path.split("/")
            for count in range(1, len(parts)):
                parent = "/".join(parts[:count])
                self.entries[parent] = {"path": parent, "kind": "directory", "uid": 0, "gid": 0, "mode": 0o700}
            mode = 0o600 if parts[-1] in ("installation-id", "epoch-floor") else 0o400
            self.entries[path] = {"path": path, "kind": "file", "uid": 0, "gid": 0,
                                  "mode": mode, "size": len(raw), "sha256": fixture_digest(raw)}

    def read(self, path, maximum=ref.MAX_DOCUMENT, metadata=None):
        if path not in self.values or len(self.values[path]) > maximum:
            raise ref.ReferenceError("REFERENCES_FIXTURE_ORIGINAL")
        if metadata is not None:
            entry = self.entries[path]
            ref.require((entry["uid"], entry["gid"], entry["mode"]) == metadata, "REFERENCES_FIXTURE_METADATA")
        return self.values[path]


def journal_history_fixture():
    encode = lambda v: json.dumps(v, sort_keys=True, ensure_ascii=True, separators=(",", ":")).encode("ascii")
    digest = fixture_digest
    generation, installation, account_id = D, C, A
    intent = {"schema": "owner-beta-paper-operation-intent-v1", "installation_id": installation,
        "generation": generation, "account_id": account_id, "predecessor_generation": None,
        "request_key": generation + ":begin", "purpose": "BACKUP", "scope": "COMBINED_BACKUP",
        "expected_epoch": 4, "local_epoch_floor": 4, "release_manifest_sha256": H,
        "artifact_sha256": None, "resume_target_id": None}
    root_raw = encode(intent)
    stage = {"schema": "owner-beta-paper-operation-stage-v1", "installation_id": installation,
        "generation": generation, "account_id": account_id, "sequence": 0, "previous_sha256": None,
        "intent_sha256": digest(root_raw), "stage": "INTENT", "boot_id": B,
        "observed": {"realtime_ns": 100, "monotonic_ns": 10, "boottime_ns": 20},
        "control_epoch": 4, "artifact_sha256": None, "event_id": None, "error_code": "NONE"}
    stage_raw = encode(stage)
    action = {"schema": "owner-beta-paper-action-binding-v1", "installation_id": installation,
        "generation": generation, "root_intent_sha256": digest(root_raw), "sequence": 0,
        "previous_binding_sha256": None, "verb": "begin", "request_key": generation + ":begin",
        "expected_epoch": 4, "artifact_sha256": None, "stage_sha256": digest(stage_raw)}
    action_raw = encode(action)
    request = {"schema": "owner-beta-paper-exact-request-v1", "kind": "BEGIN", "account_id": account_id,
        "args": {"expected_epoch": 4, "intent_canonical": root_raw.decode(), "request_key": generation + ":begin"}}
    request_raw = encode(request)
    dispatch = {"schema": "owner-beta-paper-native-dispatch-v2", "root_intent_canonical": root_raw.decode(),
        "root_intent_sha256": digest(root_raw), "action_canonical": action_raw.decode(),
        "action_sha256": digest(action_raw), "artifact_canonical": None, "artifact_sha256": None,
        "sql_request_canonical": request_raw.decode(), "sql_request_sha256": digest(request_raw)}
    dispatch_raw = encode(dispatch)
    result = {"schema": "owner-beta-paper-action-v1", "action": "BEGIN_OPERATION", "outcome": "OK",
        "control_epoch": 5, "account_id": account_id, "data": {"generation": generation,
            "operation_event_id": B, "intent_sha256": digest(root_raw), "fenced_epoch": 5}}
    result_raw = encode(result)
    root = "paper_operations/operations/" + generation
    action_root = root + "/actions/00000000"
    output = action_root + "/output"
    values = {"paper_operations/installation-id": (installation + "\n").encode(),
        "paper_operations/epoch-floor": b"5\n", root + "/intent.json": root_raw,
        root + "/00000000.json": stage_raw, root + "/" + digest(stage_raw)[7:] + ".json": stage_raw,
        action_root + "/binding.json": action_raw, action_root + "/import.json": dispatch_raw,
        output + "/dispatch.json": dispatch_raw}
    for original in (root_raw, action_raw, request_raw, dispatch_raw, result_raw):
        values[output + "/" + digest(original)[7:] + ".json"] = original
    caller_key = "fixture:backup"
    command = encode({k: v for k, v in intent.items() if k not in ("generation", "request_key")})
    caller = {"schema": "owner-beta-paper-caller-binding-v1", "installation_id": installation,
        "account_id": account_id, "caller_key": caller_key, "generation": generation,
        "root_intent_sha256": digest(root_raw), "command_canonical": command.decode()}
    caller_name = digest(encode({"installation_id": installation, "account_id": account_id,
                                 "caller_key": caller_key}))[7:] + ".json"
    values["paper_operations/callers/" + caller_name] = encode(caller)
    row = db_row("owner_beta_paper_operations", generation=generation, installation_id=installation,
        account_id=account_id, request_key=generation + ":begin", purpose="BACKUP", scope="COMBINED_BACKUP",
        expected_epoch=4, local_epoch_floor=4, fence_epoch=5, release_manifest_sha256=H,
        intent=intent, intent_sha256=digest(root_raw), prior_state="ENABLED", fence_event_id=B,
        request_canonical=request_raw.decode(), request_sha256=digest(request_raw),
        result_canonical=result_raw.decode(), result_sha256=digest(result_raw))
    return {"values": values, "row": row, "root": root, "action_root": action_root,
            "output": output, "intent": intent, "stage": stage, "action": action,
            "request": request, "dispatch": dispatch, "result": result, "caller_name": caller_name}


def journal_references(fixture):
    references = ref._References(limits(), DOC)
    references.decode(io.BytesIO(rows_stream({"owner_beta_paper_operations": [(D, fixture["row"])]})))
    return references


class PaperJournalClosureTests(unittest.TestCase):
    def test_nonempty_original_intent_action_stage_caller_and_exact_receipt_graph(self):
        fixture = journal_history_fixture()
        graph = ref._paper_journal(_JournalFixtureBytes(fixture["values"]), journal_references(fixture))
        self.assertEqual(set(graph["operations"]), {D})
        self.assertEqual(graph["requests"][fixture["row"]["request_sha256"]], (D, 0, "begin"))
        self.assertIn(fixture["row"]["result_sha256"], graph["objects"])

    def test_all_null_legacy_sql_receipts_do_not_invent_originals(self):
        fixture = journal_history_fixture()
        for key in ("request_canonical", "request_sha256", "result_canonical", "result_sha256"):
            fixture["row"][key] = None
        graph = ref._paper_journal(_JournalFixtureBytes(fixture["values"]), journal_references(fixture))
        self.assertIn(D, graph["operations"])

    def test_every_referenced_original_deletion_is_detected(self):
        fixture = journal_history_fixture()
        required = [fixture["root"] + "/intent.json", fixture["action_root"] + "/binding.json",
            fixture["output"] + "/dispatch.json",
            fixture["output"] + "/" + fixture["row"]["request_sha256"][7:] + ".json",
            fixture["output"] + "/" + fixture["row"]["result_sha256"][7:] + ".json"]
        for path in required:
            changed = copy.deepcopy(fixture)
            del changed["values"][path]
            with self.subTest(path=path), self.assertRaises(ref.ReferenceError):
                ref._paper_journal(_JournalFixtureBytes(changed["values"]), journal_references(changed))

    def test_changed_original_hash_import_stage_chain_and_floor_reject(self):
        fixture = journal_history_fixture()
        changes = [
            (fixture["action_root"] + "/import.json", b"{}"),
            (fixture["root"] + "/00000000.json", b"{}"),
            ("paper_operations/epoch-floor", b"3\n"),
            ("paper_operations/installation-id", (B + "\n").encode()),
            (fixture["output"] + "/" + fixture["row"]["result_sha256"][7:] + ".json", b'{"private":"sentinel"}'),
        ]
        for path, value in changes:
            changed = copy.deepcopy(fixture)
            changed["values"][path] = value
            with self.subTest(path=path), self.assertRaises(ref.ReferenceError) as error:
                ref._paper_journal(_JournalFixtureBytes(changed["values"]), journal_references(changed))
            self.assertNotIn("sentinel", str(error.exception))

    def test_rebound_receipt_without_retained_original_and_mixed_generation_reject(self):
        fixture = journal_history_fixture()
        changed = copy.deepcopy(fixture)
        changed["row"]["result_canonical"] = '{"private":"sentinel"}'
        changed["row"]["result_sha256"] = fixture_digest(changed["row"]["result_canonical"].encode())
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_JOURNAL_RECEIPT_MISSING$"):
            ref._paper_journal(_JournalFixtureBytes(changed["values"]), journal_references(changed))
        changed = copy.deepcopy(fixture)
        changed["row"]["installation_id"] = B
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_JOURNAL_DATABASE$"):
            ref._paper_journal(_JournalFixtureBytes(changed["values"]), journal_references(changed))

    def test_unknown_member_and_noncontiguous_action_are_not_omitted(self):
        for suffix in ("/unknown.json", "/actions/00000002/binding.json"):
            fixture = journal_history_fixture()
            fixture["values"][fixture["root"] + suffix] = b"{}"
            with self.subTest(suffix=suffix), self.assertRaises(ref.ReferenceError):
                ref._paper_journal(_JournalFixtureBytes(fixture["values"]), journal_references(fixture))

    def test_original_owner_mode_and_directory_type_are_checked(self):
        fixture = journal_history_fixture()
        for field, value in (("uid", 10001), ("gid", 10001), ("mode", 0o644)):
            reader = _JournalFixtureBytes(fixture["values"])
            reader.entries[fixture["root"] + "/intent.json"][field] = value
            with self.subTest(field=field), self.assertRaises(ref.ReferenceError):
                ref._paper_journal(reader, journal_references(fixture))

    def test_partial_or_truncated_real_decoder_cannot_reach_journal_graph(self):
        fixture = journal_history_fixture()
        stream = rows_stream({"owner_beta_paper_operations": [(D, fixture["row"])]})
        references = ref._References(limits(), DOC)
        with self.assertRaises(Exception):
            references.decode(io.BytesIO(stream[:-1]))
        self.assertTrue(references.failed)
        with self.assertRaises(ref.ReferenceError):
            ref._paper_journal(_JournalFixtureBytes(fixture["values"]), references)

    def test_pure_original_dictionary_cannot_create_pending_owner(self):
        fixture = journal_history_fixture()
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_SNAPSHOT$"):
            ref.prepare_references(_JournalFixtureBytes(fixture["values"]), io.BytesIO(rows_stream()))
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_PENDING_PRIVATE$"):
            ref._PendingSourceClosure(object(), {}, {}, {}, {}, {}, {})




    def test_jointly_replaced_result_original_and_db_hash_still_require_receipt_identity(self):
        for field, value in (("control_epoch", 6), ("account_id", B),
                             ("action", "RECONCILE_OPERATION"), ("outcome", "REPLAY")):
            fixture = journal_history_fixture()
            changed = copy.deepcopy(fixture["result"])
            changed[field] = value
            raw = json.dumps(changed, sort_keys=True, separators=(",", ":")).encode()
            fixture["row"]["result_canonical"] = raw.decode()
            fixture["row"]["result_sha256"] = fixture_digest(raw)
            fixture["values"][fixture["output"] + "/" + fixture_digest(raw)[7:] + ".json"] = raw
            with self.subTest(field=field), self.assertRaises(ref.ReferenceError):
                ref._paper_journal(_JournalFixtureBytes(fixture["values"]), journal_references(fixture))

    def test_jointly_replaced_begin_data_cannot_rebind_generation_event_or_intent(self):
        for field, value in (("generation", B), ("operation_event_id", A), ("intent_sha256", J), ("fenced_epoch", 6)):
            fixture = journal_history_fixture()
            changed = copy.deepcopy(fixture["result"])
            changed["data"][field] = value
            raw = json.dumps(changed, sort_keys=True, separators=(",", ":")).encode()
            fixture["row"]["result_canonical"] = raw.decode()
            fixture["row"]["result_sha256"] = fixture_digest(raw)
            fixture["values"][fixture["output"] + "/" + fixture_digest(raw)[7:] + ".json"] = raw
            with self.subTest(field=field), self.assertRaises(ref.ReferenceError):
                ref._paper_journal(_JournalFixtureBytes(fixture["values"]), journal_references(fixture))

class HistoricalRawCommitmentTests(unittest.TestCase):
    def retained(self):
        row = raw_batch()
        row["provider"] = "kis-daily-range"
        encoded = json.dumps(row, separators=(",", ":")).encode()
        first, second = fixture_digest(encoded), fixture_digest(b" " + encoded)
        original = fixture_digest(json.dumps(row, indent=2).encode())
        index = ref._RawIndex(ref._Budget(limits()))
        index.add(row, "raw/provider=kis-daily-range/market=kr/date=2026-11-06/batch=" + A, original, first, 1)
        index.add(row, "raw/provider=kis-daily-range/market=kr/date=2026-11-06/batch=" + A, original, second, 2)
        return index, row, original, first, second

    def test_approved_line_may_be_any_retained_source_equivalent_occurrence(self):
        index, row, original, first, second = self.retained()
        selected = ref._historical_raw_batch(index, "price_source", A, len(row["files"]), second, original)
        self.assertEqual(selected[0]["manifest_lines"], [[first, 1], [second, 2]])

    def test_missing_line_wrong_batch_content_scope_and_count_fail(self):
        for change in ("line", "body", "scope", "count"):
            index, row, original, first, second = self.retained()
            count, line, content = len(row["files"]), second, original
            if change == "line":
                line = fixture_digest(b"absent")
            elif change == "body":
                content = fixture_digest(b"other-batch")
            elif change == "scope":
                index.files = {("kis-calendar", *key[1:]): value for key, value in index.files.items()}
            else:
                count += 1
            with self.subTest(change=change), self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_HISTORICAL_RAW$"):
                ref._historical_raw_batch(index, "price_source", A, count, line, content)


class JournalBoundedIndexTests(unittest.TestCase):
    def test_event_exact_receipt_without_any_local_journal_is_rejected(self):
        fixture = journal_history_fixture()
        receipt = {key: fixture["row"][key] for key in
                   ("request_canonical", "request_sha256", "result_canonical", "result_sha256")}
        references = ref._References(limits(), DOC)
        references.decode(io.BytesIO(rows_stream({"owner_beta_paper_events": [
            (A, db_row("owner_beta_paper_events", id=A, **receipt))]})))
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_JOURNAL_MISSING$"):
            ref._paper_journal(_JournalFixtureBytes({}), references)

    def test_each_retained_hash_reference_consumes_index_budget(self):
        fixture = journal_history_fixture()
        references = journal_references(fixture)
        references.budget.entries = references.budget.used_entries + 1
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_INDEX_LIMIT$"):
            ref._paper_journal(_JournalFixtureBytes(fixture["values"]), references)

    def test_returned_exact_receipt_cannot_belong_to_other_retained_request(self):
        fixture = journal_history_fixture()
        request = json.loads(fixture["row"]["request_canonical"])
        request["generation"] = B
        raw = json.dumps(request, sort_keys=True, separators=(",", ":")).encode()
        fixture["row"]["request_canonical"] = raw.decode()
        fixture["row"]["request_sha256"] = fixture_digest(raw)
        fixture["values"][fixture["output"] + "/" + fixture_digest(raw)[7:] + ".json"] = raw
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_JOURNAL_RECEIPT_MISSING$"):
            ref._paper_journal(_JournalFixtureBytes(fixture["values"]), journal_references(fixture))


class PaperHistoryJoinTests(unittest.TestCase):
    def graph(self):
        rows = {
            "accounts": [(A, db_row("accounts", id=A, owner_user_id=B, account_type="PAPER",
                                   cost_profile_id="fixture-cost", cost_profile_version=1))],
            "user_strategy_configs": [(C, db_row("user_strategy_configs", id=C, owner_user_id=B,
                strategy_id="fixture-strategy", strategy_version="1", config_json={}))],
            "account_strategy_bindings": [(D, db_row("account_strategy_bindings", id=D,
                account_id=A, owner_user_id=B, strategy_config_id=C,
                strategy_id="fixture-strategy", strategy_version="1"))],
            "owner_beta_paper_events": [(D, db_row("owner_beta_paper_events", id=D, account_id=A, owner_user_id=B))],
            "owner_beta_paper_control": [("fixture-control", db_row("owner_beta_paper_control",
                id="fixture-control", owner_user_id=B, account_id=A, strategy_config_id=C,
                cost_profile_id="fixture-cost", cost_profile_version=1,
                consent_event_id=D, activation_event_id=D))],
            "owner_beta_paper_lineage": [(D, db_row("owner_beta_paper_lineage", id=D,
                account_id=A, owner_user_id=B, strategy_config_id=C, effective_session="2026-11-06"))],
            "pending_targets": [(C, db_row("pending_targets", id=C, account_id=A, owner_user_id=B,
                strategy_config_id=C, beta_lineage_id=D, effective_date="2026-11-06"))],
            "owner_beta_paper_sessions": [(C, db_row("owner_beta_paper_sessions", pending_target_id=C,
                account_id=A, owner_user_id=B, effective_session="2026-11-06"))],
            "paper_rebalance_previews": [(D, db_row("paper_rebalance_previews", id=D, account_id=A,
                owner_user_id=B, strategy_config_id=C))],
            "owner_beta_paper_recovery_grants": [(D, db_row("owner_beta_paper_recovery_grants",
                grant_id=D, account_id=A, pending_target_id=C, lineage_id=D, effective_session="2026-11-06"))],
            "data_entitlements": [(A, db_row("data_entitlements", id=A, status="REVOKED",
                contract_document_sha256=H, contract_reference="fixture-rights-history"))],
        }
        refs = ref._References(limits(), DOC)
        refs.decode(io.BytesIO(rows_stream(rows)))
        return refs

    def test_nonempty_session_lineage_config_and_revoked_rights_joins(self):
        refs = self.graph()
        ref._paper_relationships(refs, {})
        self.assertEqual(next(iter(refs.rows("data_entitlements")))["status"], "REVOKED")

    def test_cross_account_owner_and_strategy_binding_fail(self):
        for table, field, value in (("accounts", "owner_user_id", C),
            ("user_strategy_configs", "owner_user_id", A),
            ("account_strategy_bindings", "strategy_version", "other"),
            ("owner_beta_paper_sessions", "account_id", C),
            ("paper_rebalance_previews", "strategy_config_id", D)):
            refs = self.graph()
            next(iter(refs.rows(table)))[field] = value
            with self.subTest(table=table, field=field), self.assertRaises(ref.ReferenceError):
                ref._paper_relationships(refs, {})

    def test_deleted_target_event_and_lineage_cannot_be_normalized_away(self):
        for table in ("pending_targets", "owner_beta_paper_events", "owner_beta_paper_lineage"):
            refs = self.graph()
            refs.tables[table].clear()
            with self.subTest(table=table), self.assertRaises(ref.ReferenceError):
                ref._paper_relationships(refs, {})

    def test_changed_cost_session_date_recovery_and_missing_admission_fail(self):
        for table, field, value in (
            ("owner_beta_paper_control", "cost_profile_version", 2),
            ("owner_beta_paper_sessions", "effective_session", "2026-11-07"),
            ("owner_beta_paper_recovery_grants", "lineage_id", C),
            ("owner_beta_paper_sessions", "execution_admission_sha256", J),
            ("owner_beta_paper_recovery_grants", "signal_admission_sha256", J)):
            refs = self.graph()
            next(iter(refs.rows(table)))[field] = value
            with self.subTest(table=table, field=field), self.assertRaises(ref.ReferenceError):
                ref._paper_relationships(refs, {})

class HistoricalRunClosureTests(unittest.TestCase):
    def history(self):
        root = Path(__file__).resolve().parents[2]
        v2_raw = (root / "configs/evidence/kis-historical-price-only-beta-approved-artifacts.json").read_bytes()
        v3_raw = (root / "configs/evidence/kis-historical-price-only-v3-approved-artifacts.json").read_bytes()
        v2, v3 = (json.loads(raw)["approved_artifacts"][0] for raw in (v2_raw, v3_raw))
        current = {key: v3[key] for key in ref._HISTORICAL_PIN_FIELDS[:-1]}
        current.update(approval_registry_sha256=fixture_digest(v3_raw), bars_sha256=v3["bars_sha256"])
        reader = _JournalFixtureBytes({"simulation_approval/" + ref.LEGACY_APPROVAL_NAME: v2_raw,
                                      "simulation_approval/" + ref.simulation.APPROVAL_NAME: v3_raw})
        for name in (ref.LEGACY_APPROVAL_NAME, ref.simulation.APPROVAL_NAME):
            reader.entries["simulation_approval/" + name]["mode"] = 0o644
        rows, jobs = [], []
        for identity, job, record, registry in ((A, C, v2, v2_raw), (B, D, v3, v3_raw)):
            values = {key: record[key] for key in ref._HISTORICAL_PIN_FIELDS[:-1]}
            values["approval_registry_sha256"] = fixture_digest(registry)
            rows.append((identity, db_row("owner_beta_recommendation_runs", id=identity, job_id=job,
                owner_user_id=B, **values)))
            jobs.append((job, db_row("jobs", id=job, owner_user_id=B)))
        references = ref._References(limits(), DOC)
        references.decode(io.BytesIO(rows_stream({"owner_beta_recommendation_runs": rows, "jobs": jobs})))
        return reader, references, current

    def test_distinct_adopted_v2_v3_history_is_not_forced_to_current_tuple(self):
        reader, references, current = self.history()
        ref._historical_run_pins(reader, references, current)
        self.assertNotEqual(next(iter(references.rows("owner_beta_recommendation_runs")))["approval_registry_sha256"],
                            current["approval_registry_sha256"])

    def test_mixed_old_new_pins_missing_registry_and_cross_owner_job_reject(self):
        for change in ("tuple", "registry", "job", "owner"):
            reader, references, current = self.history()
            old = next(iter(references.rows("owner_beta_recommendation_runs")))
            if change == "tuple":
                old["artifact_manifest_sha256"] = current["artifact_manifest_sha256"]
            elif change == "registry":
                reader.entries.clear()
            elif change == "job":
                del references.jobs[C]
            else:
                references.jobs[C]["owner_user_id"] = A
            with self.subTest(change=change), self.assertRaises(ref.ReferenceError):
                ref._historical_run_pins(reader, references, current)

    def test_changed_legacy_registry_bytes_are_not_replaced_by_matching_hash_claim(self):
        reader, references, current = self.history()
        reader.values["simulation_approval/" + ref.LEGACY_APPROVAL_NAME] += b"\n"
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_LEGACY_APPROVAL$"):
            ref._historical_run_pins(reader, references, current)

    def test_unrelated_simulation_published_attempt_is_rejected(self):
        reader, references, current = self.history()
        references.tables["owner_beta_simulation_runs"][A] = db_row("owner_beta_simulation_runs",
            id=A, owner_user_id=B, job_id=C, published_attempt_id=D, **current)
        references.attempts[D] = {"id": D, "job_id": A}
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_HISTORY_ATTEMPT$"):
            ref._historical_run_pins(reader, references, current)

    def test_current_hash_tuple_cannot_substitute_for_missing_v3_original(self):
        reader, references, current = self.history()
        del reader.entries["simulation_approval/" + ref.simulation.APPROVAL_NAME]
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_HISTORICAL_RUN_PIN$"):
            ref._historical_run_pins(reader, references, current)

    def test_uninstalled_current_r4_does_not_hide_old_recommendation_pins(self):
        reader, references, _ = self.history()
        ref._historical_run_pins(reader, references, None)
        del reader.entries["simulation_approval/" + ref.LEGACY_APPROVAL_NAME]
        with self.assertRaisesRegex(ref.ReferenceError, "^REFERENCES_HISTORICAL_RUN_PIN$"):
            ref._historical_run_pins(reader, references, None)


class DatasetCaptureTests(unittest.TestCase):
    def vector(self):
        return json.loads((PATH.parents[3] / "tests/fixtures/owner-beta-paper-archive-framing-v1.json").read_text())

    def test_shared_unicode_golden_frame_and_dataset_capture(self):
        vector = self.vector()
        root = ref._FramedRoot()
        frames = b""
        for table, key, row in vector["records"]:
            root.add(table, key, row)
            frames += ref.audit.frame(table, key, row)
        self.assertEqual(frames.hex(), vector["framed_hex"])
        self.assertEqual(root._description(), vector["description"])
        self.assertEqual(ref.encode_dataset_rows(vector["dataset_rows"]), vector["dataset_jsonl"].encode())
        self.assertEqual(ref.encode_dataset_rows([]), b"")

    def test_dataset_capture_exact_shape_order_and_limits(self):
        row = self.vector()["dataset_rows"][0]
        for bad in ({**row, "extra": "x"}, {k:v for k,v in row.items() if k != "status"},
                    {**row, "version": 1}, {**row, "id": "bad"},
                    {**row, "manifest_sha256": H}, {**row, "version": "x" * ref.MAX_LINE}):
            with self.subTest(bad_keys=sorted(bad)), self.assertRaises(ref.ReferenceError):
                ref.encode_dataset_rows([bad])
        with self.assertRaises(ref.ReferenceError):
            ref.encode_dataset_rows([row, row])
        from unittest.mock import patch
        with patch.object(ref, "MAX_DATASET_BYTES", 10), self.assertRaises(ref.ReferenceError):
            ref.encode_dataset_rows([row])

    def test_preview_only_capture_and_present_target_version(self):
        captured = self.vector()["dataset_rows"]
        row = db_row("paper_rebalance_previews", id=B, dataset_version_id=A,
                     dataset_manifest_sha256="a" * 64)
        references = ref._References(limits(), DOC)
        references.decode(io.BytesIO(rows_stream({"paper_rebalance_previews": [(B, row)]})))
        self.assertEqual(ref._dataset_rows(references, captured), captured)
        for field in ("id", "manifest_sha256", "storage_path"):
            bad = copy.deepcopy(captured)
            bad[0][field] = {"id": B, "manifest_sha256": "b" * 64, "storage_path": "/tmp/curated"}[field]
            with self.subTest(field=field), self.assertRaises(ref.ReferenceError):
                ref._dataset_rows(references, bad)
        target = db_row("pending_targets", id=C, dataset_version_id=A,
                        dataset_manifest_sha256="a" * 64, dataset_version="different")
        references = ref._References(limits(), DOC)
        references.decode(io.BytesIO(rows_stream({"paper_rebalance_previews": [(B, row)],
                                                  "pending_targets": [(C, target)]})))
        with self.assertRaises(ref.ReferenceError):
            ref._dataset_rows(references, captured)


if __name__ == "__main__":
    unittest.main()
