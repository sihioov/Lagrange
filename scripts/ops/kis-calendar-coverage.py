#!/usr/bin/env python3
"""Inventory committed KIS calendar dates without provider or database access.

This reports the observed Raw calendar coverage. The native supplier remains
the authority for coverage proof and action planning.
"""

import argparse
from collections import Counter
from contextlib import ExitStack, contextmanager
from dataclasses import dataclass
from datetime import date, datetime, timedelta, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import uuid


PROVIDERS = ("kis", "kis-calendar")
START = date(2026, 8, 29)
DAY = timedelta(days=1)
KST = timezone(timedelta(hours=9))
MAX_DAYS = 370
MANIFEST_LIMIT = 8_388_608
BATCH_LIMIT = 262_144
BODY_LIMIT = 131_072
MAX_BATCHES = 128
ENDPOINT = "/uapi/domestic-stock/v1/quotations/chk-holiday"
HEADERS = {
    "authorization": "[REDACTED]",
    "appkey": "[REDACTED]",
    "appsecret": "[REDACTED]",
    "tr_id": "CTCA0903R",
    "tr_cont": "",
}


class InventoryError(Exception):
    pass


def require(condition, code):
    if not condition:
        raise InventoryError(code)


def decode(wire):
    def unique_pairs(pairs):
        result = dict(pairs)
        require(len(result) == len(pairs), "DUPLICATE_JSON_KEY")
        return result

    def invalid_constant(_):
        raise InventoryError("INVALID_JSON")

    return json.loads(wire.decode("utf-8"), object_pairs_hook=unique_pairs,
                      parse_constant=invalid_constant)


def digest(wire):
    return "sha256:" + hashlib.sha256(wire).hexdigest()


def iso_date(value):
    require(isinstance(value, str) and re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", value),
            "INVALID_DATE")
    return date.fromisoformat(value)


def row_date(value):
    require(isinstance(value, str) and re.fullmatch(r"[0-9]{8}", value), "INVALID_CALENDAR_ROW")
    return date(int(value[:4]), int(value[4:6]), int(value[6:]))


def safe_leaf(value):
    return (isinstance(value, str) and ".." not in value
            and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}", value) is not None)


@contextmanager
def open_at(root_fd, parts, *, directory=False):
    """Read only through direct path components, without following symlinks."""
    with ExitStack() as stack:
        fd = root_fd
        for index, part in enumerate(parts):
            require(part not in {"", ".", ".."} and "/" not in part, "INVALID_PATH")
            is_directory = directory or index + 1 < len(parts)
            flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK
            fd = os.open(part, flags | (os.O_DIRECTORY if is_directory else 0), dir_fd=fd)
            stack.callback(os.close, fd)
        if not directory:
            info = os.fstat(fd)
            require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1, "INVALID_RAW_FILE")
        yield fd


def read_at(root_fd, parts, limit):
    with open_at(root_fd, parts) as fd, os.fdopen(fd, "rb", closefd=False) as source:
        wire = source.read(limit + 1)
        require(len(wire) <= limit, "RAW_LIMIT_EXCEEDED")
        return wire


def exact_pairs(pairs):
    require(isinstance(pairs, list) and all(
        isinstance(pair, list) and len(pair) == 2 and all(isinstance(s, str) for s in pair)
        for pair in pairs
    ), "INVALID_CALENDAR_REQUEST")
    result = dict(pairs)
    require(len(result) == len(pairs), "INVALID_CALENDAR_REQUEST")
    return result


@dataclass(frozen=True)
class Response:
    partition_date: date
    retrieved_at: datetime
    metadata: dict
    request_date: date
    days: dict[date, str]

    def matches(self, other):
        fields = ("content_hash", "size_bytes", "request", "response_continuation")
        return all(self.metadata.get(key) == other.metadata.get(key) for key in fields)


def load_response(raw_fd, provider, batch, metadata, observed_at):
    partition_date = iso_date(batch["date"])
    batch_id = batch["batch_id"]
    parsed_id = uuid.UUID(batch_id)
    require(str(parsed_id) == batch_id and parsed_id.int != 0, "INVALID_BATCH_ID")
    retrieved_at = datetime.fromisoformat(batch["retrieved_at"])
    require(retrieved_at.tzinfo is not None and retrieved_at <= observed_at, "INVALID_RETRIEVAL")
    directory = (f"provider={provider}", "market=kr", f"date={partition_date}", f"batch={batch_id}")
    require(decode(read_at(raw_fd, (*directory, "batch.json"), BATCH_LIMIT)) == batch,
            "BATCH_METADATA_MISMATCH")
    file_name = metadata.get("file_name")
    require(batch["mode"] == "credentialed" and safe_leaf(file_name) and file_name != "batch.json",
            "INVALID_CALENDAR_SOURCE")
    marker = metadata.get("response_continuation")
    require(marker is None or isinstance(marker, str) and re.fullmatch(r"[A-Za-z0-9]{0,8}", marker),
            "INVALID_CALENDAR_SOURCE")
    body = read_at(raw_fd, (*directory, file_name), BODY_LIMIT)
    require(type(metadata["size_bytes"]) is int and metadata["size_bytes"] == len(body)
            and metadata["content_hash"] == digest(body), "CALENDAR_HASH_MISMATCH")
    request = metadata["request"]
    query = exact_pairs(request["query"])
    require(request["endpoint"] == ENDPOINT and request["mode"] == "credentialed"
            and exact_pairs(request["headers"]) == HEADERS
            and set(query) == {"BASS_DT", "CTX_AREA_FK", "CTX_AREA_NK"}
            and query["CTX_AREA_FK"] == query["CTX_AREA_NK"] == "", "INVALID_CALENDAR_REQUEST")
    request_date = row_date(query["BASS_DT"])
    document = decode(body)
    require(isinstance(document, dict) and document.get("rt_cd") == "0", "INVALID_CALENDAR_BODY")
    rows = document.get("output")
    if isinstance(rows, dict):
        rows = [rows]
    require(isinstance(rows, list) and 0 < len(rows) <= MAX_DAYS, "INVALID_CALENDAR_BODY")
    days = {}
    for row in rows:
        require(isinstance(row, dict), "INVALID_CALENDAR_ROW")
        day = row_date(row.get("bass_dt"))
        opened = row.get("opnd_yn")
        require(opened in ("Y", "N") and day not in days, "INVALID_CALENDAR_ROW")
        days[day] = opened
    require(request_date in days and (max(days) - min(days)).days < MAX_DAYS,
            "INVALID_CALENDAR_WINDOW")
    return Response(partition_date, retrieved_at, metadata, request_date, days)


def responses(raw_fd, entitlement_hash, observed_at):
    result = {}
    with ExitStack() as stack:
        for provider in PROVIDERS:
            scope = ("manifests", f"provider={provider}", "market=kr")
            lock = stack.enter_context(open_at(raw_fd, (*scope, "commit.lock")))
            fcntl.flock(lock, fcntl.LOCK_SH | fcntl.LOCK_NB)
        for provider in PROVIDERS:
            scope = ("manifests", f"provider={provider}", "market=kr")
            wire = read_at(raw_fd, (*scope, "manifest.jsonl"), MANIFEST_LIMIT)
            require(not wire or wire.endswith(b"\n"), "INCOMPLETE_MANIFEST")
            require(wire.count(b"\n") <= MAX_BATCHES, "RAW_LIMIT_EXCEEDED")
            seen = set()
            for line in wire.split(b"\n")[:-1]:
                require(len(line) <= BATCH_LIMIT, "RAW_LIMIT_EXCEEDED")
                record = decode(line)
                batch_id = record["batch_id"]
                require(record["provider"] == provider and record["market"] == "kr"
                        and batch_id not in seen, "INVALID_MANIFEST")
                seen.add(batch_id)
                entitlement = record.get("entitlement_reference")
                if not isinstance(entitlement, str) or digest(entitlement.encode()) != entitlement_hash:
                    continue
                files = [file for file in record["files"] if file["kind"] == "calendar"]
                require(len(files) <= 1, "INVALID_CALENDAR_SOURCE")
                if files:
                    metadata = files[0]
                    candidate = load_response(raw_fd, provider, record, metadata, observed_at)
                    result[provider, batch_id, metadata["file_name"]] = candidate
    return result


def original_responses(records):
    originals = []
    cached = []
    for key, source in records.items():
        copied_from = source.metadata.get("copied_from")
        if copied_from is None:
            if source.request_date == source.partition_date:
                originals.append((key[0], source))
            else:
                cached.append((key[0], source))
            continue
        seen = {key}
        current = source
        while copied_from is not None:
            origin_key = (copied_from["provider"], copied_from["batch_id"], copied_from["file_name"])
            require(origin_key not in seen and len(seen) <= 16, "INVALID_CALENDAR_COPY")
            seen.add(origin_key)
            origin = records.get(origin_key)
            require(origin is not None and origin.matches(source)
                    and origin.retrieved_at <= current.retrieved_at
                    and copied_from["market"] == "kr"
                    and copied_from["content_hash"] == origin.metadata["content_hash"]
                    and datetime.fromisoformat(copied_from["retrieved_at"]) == origin.retrieved_at,
                    "INVALID_CALENDAR_COPY")
            current = origin
            copied_from = origin.metadata.get("copied_from")
        require(current.request_date == current.partition_date, "INVALID_CALENDAR_COPY")
    for provider, source in cached:
        require(provider == "kis" and any(
            original_provider == provider and original.partition_date < source.partition_date
            and original.retrieved_at <= source.retrieved_at and original.matches(source)
            and original.metadata["file_name"] == source.metadata["file_name"]
            for original_provider, original in originals
        ), "INVALID_LEGACY_CALENDAR_CACHE")
    require(originals, "NO_MATCHING_CALENDAR_ORIGINALS")
    return [source for _, source in originals]


def inventory(data_root, entitlement_hash, through, observed_at):
    require(data_root.is_absolute() and data_root.anchor == "/", "INVALID_PATH")
    require(re.fullmatch(r"sha256:[0-9a-f]{64}", entitlement_hash), "INVALID_ENTITLEMENT_HASH")
    require(0 <= (through - START).days < MAX_DAYS, "INVALID_REQUIRED_RANGE")
    with ExitStack() as stack:
        system_root = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
        stack.callback(os.close, system_root)
        with open_at(system_root, (*data_root.parts[1:], "raw"), directory=True) as raw_fd:
            deliveries = responses(raw_fd, entitlement_hash, observed_at)
    originals = original_responses(deliveries)
    by_day = {}
    windows = Counter()
    for source in originals:
        windows[source.request_date, min(source.days), max(source.days), len(source.days)] += 1
        for day, opened in source.days.items():
            require(by_day.setdefault(day, opened) == opened, "CALENDAR_CONFLICT")

    missing = []
    missing_days = 0
    for offset in range((through - START).days + 1):
        day = START + timedelta(days=offset)
        if day in by_day:
            continue
        missing_days += 1
        if missing and (day - missing[-1][1]).days == 1:
            missing[-1][1] = day
        else:
            missing.append([day, day])
    return {
        "status": "inventory",
        "required_start": START.isoformat(),
        "required_end": through.isoformat(),
        "missing_days": missing_days,
        "missing_intervals": [
            {"start": first.isoformat(), "end": last.isoformat()} for first, last in missing
        ],
        "original_responses": len(originals),
        "response_windows": [
            {"request_date": target.isoformat(), "first_date": first.isoformat(),
             "last_date": last.isoformat(), "unique_dates": row_count, "responses": response_count}
            for (target, first, last, row_count), response_count in sorted(windows.items())
        ],
    }


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise InventoryError("INVALID_ARGUMENT")


def main():
    parser = Parser(description=__doc__)
    parser.add_argument("--data-root", type=Path, required=True, help="absolute data root containing raw/")
    parser.add_argument("--entitlement-sha256", required=True, help="configured entitlement SHA-256")
    parser.add_argument("--through", help="inclusive calendar date, defaults to the 16:30 KST cutoff")
    try:
        args = parser.parse_args()
        observed_at = datetime.now(timezone.utc)
        local = observed_at.astimezone(KST)
        eligible = local.date() if (local.hour, local.minute) >= (16, 30) else local.date() - DAY
        through = eligible if args.through is None else iso_date(args.through)
        require(through <= eligible, "INVALID_REQUIRED_RANGE")
        result = inventory(args.data_root, args.entitlement_sha256, through, observed_at)
    except InventoryError as error:
        print(json.dumps({"status": "blocked", "error_code": str(error)}))
        return 2
    except BlockingIOError:
        print(json.dumps({"status": "blocked", "error_code": "RAW_COMMIT_LOCK_BUSY"}))
        return 2
    except (OSError, ValueError, KeyError, TypeError, AttributeError, RecursionError):
        print(json.dumps({"status": "blocked", "error_code": "RAW_INVENTORY_UNAVAILABLE"}))
        return 2
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
