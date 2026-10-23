#!/usr/bin/env python3
"""Synthetic extension for the host-local, actual-approved-base whole-flow test.

No provider transport, database, approval registry or clock override. All Raw
bytes are constructed before either producer invocation. The deliberately
synthetic calendar has exactly two trading sessions; it is NOT a KRX calendar.
The real executable must independently approve the separately mounted V3 base.
"""
from __future__ import annotations

import datetime as dt
import hashlib
import json
import re
import uuid

SCHEMA = "lagrange-daily-wholeflow-fixture-v1"
FIRST = dt.date(2026, 8, 31)
START = dt.date(2026, 8, 29)
KST = dt.timezone(dt.timedelta(hours=9))
SYMBOLS = ("069500", "102110", "114260", "132030", "133690", "143850",
           "148070", "153130", "192090", "195930", "229200")
ENTITLEMENT = "fixture-only:synthetic-wholeflow-extension-not-market-receipt"
POLICY_HASH = "sha256:e9149ef56170877e5e9646f1fff1c370b3a34bf2c8b8ee8272bc30bd5fa8779c"
ACTIONS = (
    ("paidin-subscription", "paidin-capin", "HHKDB669100C0", [("GB1", "1")]),
    ("paidin-record", "paidin-capin", "HHKDB669100C0", [("GB1", "2")]),
    ("bonus", "bonus-issue", "HHKDB669101C0", []),
    ("dividend", "dividend", "HHKDB669102C0", [("GB1", "0"), ("HIGH_GB", "")]),
    ("merger-split", "merger-split", "HHKDB669104C0", []),
    ("reverse-split", "rev-split", "HHKDB669105C0", [("MARKET_GB", "0")]),
    ("capital-decrease", "cap-dcrs", "HHKDB669106C0", []),
)
SERVICES = ("db-role-bootstrap", "db-migrate", "api-server", "web", "research-worker",
            "recommendation-runner", "candidate-runner", "owner-beta-runner",
            "owner-equity-v2-runner", "nt-backtest-worker-1", "nt-backtest-worker-2",
            "paper-scheduler")


class FixtureError(Exception):
    """Fixed diagnostics only."""


def require(condition, code="WHOLEFLOW_FIXTURE_INPUT"):
    if not condition:
        raise FixtureError(code)


def canonical(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=True,
                      allow_nan=False).encode("ascii")


def digest(value):
    return "sha256:" + hashlib.sha256(value).hexdigest()


def eligible(now):
    require(isinstance(now, dt.datetime) and now.utcoffset() is not None)
    local = now.astimezone(KST)
    return local.date() - dt.timedelta(days=local.time() < dt.time(16, 30))


def image_manifest(data, commit):
    """Closed V2 grammar; Docker IDs/revisions are checked by the caller too."""
    require(re.fullmatch(r"[0-9a-f]{40}", commit) and commit != "0" * 40)
    require(type(data) is bytes and len(data) <= 16384 and data.endswith(b"\n"))
    require(all(byte == 10 or 32 <= byte <= 126 for byte in data))
    lines = data.decode("ascii").splitlines()
    require(len(lines) == 14 and lines[:2] == ["LAGRANGE_RELEASE_MANIFEST_V2", "commit|" + commit])
    images = {}
    for service, line in zip(SERVICES, lines[2:], strict=True):
        fields = line.split("|")
        require(len(fields) == 5 and fields[:3] == ["image", service,
                "lagrange-station-" + service + ":" + commit] and fields[4] == commit)
        require(re.fullmatch(r"sha256:[0-9a-f]{64}", fields[3]))
        images[service] = fields[3]
    return images


def plan(now, owner, commit):
    require(isinstance(now, dt.datetime) and now.utcoffset() is not None)
    require(str(uuid.UUID(owner)) == owner and uuid.UUID(owner).int != 0)
    require(re.fullmatch(r"[0-9a-f]{40}", commit) and commit != "0" * 40)
    last = eligible(now)
    require(FIRST < last and (last - START).days < 370, "WHOLEFLOW_CALENDAR_RANGE")
    # Leave room for the bounded harness. No synthetic timestamp may stand in
    # for elapsed production polling time or an actual trading-day observation.
    rollover = dt.datetime.combine(last + dt.timedelta(days=1), dt.time(16, 30), KST)
    require((rollover - now).total_seconds() >= 1800, "WHOLEFLOW_CUTOFF_TOO_CLOSE")
    observed = now.astimezone(dt.timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")
    days = [START + dt.timedelta(days=n) for n in range((last - START).days + 1)]
    calendar = {"rt_cd": "0", "output": [
        {"bass_dt": day.strftime("%Y%m%d"), "opnd_yn": "Y" if day in (FIRST, last) else "N"}
        for day in days]}
    files = {}
    manifests = {"kis": [], "kis-calendar": []}

    def batch(provider, day, specifications):
        batch_id = str(uuid.uuid5(uuid.UUID(owner), provider + ":" + day.isoformat()))
        root = f"provider={provider}/market=kr/date={day}/batch={batch_id}"
        entries = []
        for kind, name, endpoint, tr_id, query, body in specifications:
            data = canonical(body)
            files[root + "/" + name] = data
            entries.append({"kind": kind, "file_name": name, "content_hash": digest(data),
                            "size_bytes": len(data), "request": {
                                "endpoint": "/uapi/domestic-stock/v1/" + endpoint,
                                "query": query, "headers": [["appkey", "[REDACTED]"],
                                    ["appsecret", "[REDACTED]"], ["authorization", "[REDACTED]"],
                                    ["tr_cont", ""], ["tr_id", tr_id]], "mode": "credentialed"},
                            "response_continuation": "F" if kind == "corporate_actions" else None,
                            "copied_from": None})
        entry = {"batch_id": batch_id, "provider": provider, "market": "kr", "date": day.isoformat(),
                 "retrieved_at": observed, "mode": "credentialed",
                 "entitlement_reference": ENTITLEMENT, "files": entries}
        data = canonical(entry)
        files[root + "/batch.json"] = data
        manifests[provider].append(data)

    previous = START
    for ordinal, day in enumerate((FIRST, last)):
        compact = day.strftime("%Y%m%d")
        specifications = []
        for symbol in SYMBOLS:
            specifications.append(("bars", f"bar-{symbol}.json", "quotations/inquire-daily-itemchartprice",
                "FHKST03010100", [["FID_COND_MRKT_DIV_CODE", "J"], ["FID_INPUT_ISCD", symbol],
                    ["FID_INPUT_DATE_1", compact], ["FID_INPUT_DATE_2", compact],
                    ["FID_PERIOD_DIV_CODE", "D"], ["FID_ORG_ADJ_PRC", "1"]],
                {"rt_cd": "0", "output1": {"prdt_name": "SYNTHETIC_WHOLEFLOW"}, "output2": [{
                    "stck_bsop_date": compact, "stck_oprc": "100", "stck_hgpr": "120", "stck_lwpr": "90",
                    "stck_clpr": str(110 + ordinal), "acml_vol": "200", "acml_tr_pbmn": str((110 + ordinal) * 200)}]}))
            specifications.append(("reference", f"ref-{symbol}.json", "quotations/inquire-price",
                "FHKST01010100", [["FID_COND_MRKT_DIV_CODE", "J"], ["FID_INPUT_ISCD", symbol]],
                {"rt_cd": "0", "output": {"stck_shrn_iscd": symbol}}))
        for label, path, tr_id, extras in ACTIONS:
            specifications.append(("corporate_actions", f"corporate-actions-{label}-page-01.json",
                "ksdinfo/" + path, tr_id, [["F_DT", previous.strftime("%Y%m%d")], ["T_DT", compact],
                    ["SHT_CD", ""], ["CTS", ""], *extras], {"rt_cd": "0", "output1": []}))
        if day == last:
            specifications.append(("calendar", "calendar.json", "quotations/chk-holiday", "CTCA0903R",
                [["BASS_DT", compact], ["CTX_AREA_FK", ""], ["CTX_AREA_NK", ""]], calendar))
        batch("kis", day, specifications)
        previous = day + dt.timedelta(days=1)
    batch("kis-calendar", last, [("calendar", "calendar.json", "quotations/chk-holiday", "CTCA0903R",
        [["BASS_DT", last.strftime("%Y%m%d")], ["CTX_AREA_FK", ""], ["CTX_AREA_NK", ""]], calendar)])
    for provider, rows in manifests.items():
        root = f"manifests/provider={provider}/market=kr"
        files[root + "/commit.lock"] = b""
        files[root + "/manifest.jsonl"] = b"".join(row + b"\n" for row in rows)
    config = {"schema": "owner-beta-daily-process-v1", "owner": owner, "policy_sha256": POLICY_HASH,
              "policy_path": "/run/owner-beta-daily/policy.json", "historical_root": "/data/owner-beta-artifacts",
              "raw_root": "/data/raw", "candidate_root": "/data/owner-beta-daily/candidates",
              "admitted_root": "/data/owner-beta-daily/admitted", "frontier_root": "/data/owner-beta-daily/frontiers",
              "producer_state_root": "/data/owner-beta-daily/producer-state",
              "admitter_state_root": "/data/owner-beta-daily/admitter-state", "raw_owner_uid": 10001,
              "scan": {"max_manifest_bytes": 1048576, "max_manifest_records": 8, "max_batches": 8,
                       "max_files_per_batch": 64, "max_files": 128, "max_batch_metadata_bytes": 1048576,
                       "max_source_file_bytes": 1048576, "max_retained_bytes": 16777216,
                       "max_open_descriptors": 256, "max_data_root_bytes": 4096,
                       "max_data_root_components": 64, "max_metadata_json_depth": 32},
              "max_artifact_entries": 4096, "max_replay_admissions": 8, "max_cycle_seconds": 300,
              "code_commit": commit, "entitlement_reference": ENTITLEMENT,
              "entitlement_sha256": digest(ENTITLEMENT.encode()), "first_auto_session": FIRST.isoformat()}
    return {"schema": SCHEMA, "synthetic": True, "sessions": [FIRST.isoformat(), last.isoformat()],
            "observed_at": observed, "config": config, "raw_files": files}


def audit_admissions(rows, sessions):
    require(type(rows) is list and len(rows) == 2, "WHOLEFLOW_ADMISSIONS")
    for index, row in enumerate(rows):
        require(row["range_start"] == "2016-08-29" and row["range_end"] == sessions[index]
                and row["session_count"] == 2453 + index and row["bar_count"] == 11 * (2453 + index)
                and row["parent_admission_sha256"] == (None if index == 0 else rows[0]["admission_sha256"]),
                "WHOLEFLOW_ADMISSION_CHAIN")
        for key in ("admission_sha256", "content_sha256", "manifest_sha256"):
            require(re.fullmatch(r"sha256:[0-9a-f]{64}", row[key]), "WHOLEFLOW_ADMISSION_HASH")
    require(rows[0]["admission_sha256"] != rows[1]["admission_sha256"], "WHOLEFLOW_ADMISSION_IDENTITY")


def audit_publication(state, run, admission, session):
    require(state["enabled"] is True and state["runtime_enabled"] is True
            and state["state"] == "CURRENT" and state["reason_code"] is None
            and state["expected_session"] == state["data_support_end"] == state["recommendation_as_of"] == session
            and state["input_kind"] == "owner_beta_daily_etf11_price_only_v1"
            and state["trigger_kind"] == "DAILY" and state["published_at"] is not None,
            "WHOLEFLOW_HTTP_PUBLICATION")
    require(run["id"] == state["current_reservation_run_id"] and run["status"] == "SUCCEEDED"
            and run["as_of"] == session and run["trigger_kind"] == "DAILY"
            and run["admission_sha256"] == admission["admission_sha256"]
            and run["factor_snapshot_sha256"] is not None and run["target_snapshot_sha256"] is not None,
            "WHOLEFLOW_PUBLICATION_BINDING")
