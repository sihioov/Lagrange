"""Pure stream/graph vectors. Real protected FD tests live in the Root suite."""
import copy
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
        doc = document("calendar", day, [files["calendar.json"]], 1)
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


class _FixtureBytes:
    """Pure domain byte fixtures; the production entry point rejects this type."""
    def __init__(self, values):
        self.values = values

    def read(self, path, maximum, metadata=None):
        value = self.values[path]
        if len(value) > maximum:
            raise ref.ReferenceError("REFERENCES_DOCUMENT_SIZE")
        return value


class DailyOriginalTests(unittest.TestCase):
    def test_verified_poll_cannot_omit_admission_and_frontier(self):
        row = db_row("owner_beta_daily_poll_receipts", receipt_id=A, stage="ADMITTER", owner_user_id=B,
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


if __name__ == "__main__":
    unittest.main()
