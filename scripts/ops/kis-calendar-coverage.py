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
import io
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
CAPACITY_RECORD_INSPECTION_LIMIT = 16_384
CAPACITY_FILE_INSPECTION_LIMIT = 65_536
PAPER_DESCRIPTOR_LIMIT = 5_632
CAPACITY_LIMITS = {
    "manifest_records": MAX_BATCHES,
    "batches": MAX_BATCHES,
    "largest_batch_files": 256,
    "files": 8192,
    "manifest_bytes": MANIFEST_LIMIT,
    "live_descriptors_projected": 16_384,
    "paper_archive_descriptors_projected": PAPER_DESCRIPTOR_LIMIT,
}
LIVE_DATA_ROOT_COMPONENTS = 1  # /data, the parent of the mounted /data/raw.
ARCHIVE_DATA_ROOT_COMPONENTS = 4  # /run/owner-beta-paper-ops/archive/files.
ARCHIVE_COORDINATION_COMPONENTS = 4  # /run/owner-beta-paper-ops/archive/coordination.
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


def capacity_file_name(value):
    """Mirror the native Raw file-name envelope without opening provider files."""
    if not isinstance(value, str) or not value or value in {".", "..", "batch.json"}:
        return False
    try:
        encoded = value.encode("utf-8")
    except UnicodeEncodeError:
        return False
    return (len(encoded) <= 255 and "/" not in value and "\\" not in value
            and ":" not in value and not value.endswith((".", " "))
            and not any(byte < 32 or byte == 127 for byte in encoded))


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
    with shared_manifest_locks(raw_fd):
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


@contextmanager
def shared_manifest_locks(raw_fd):
    """Hold nonblocking shared locks for both Raw scopes while reading manifests."""
    with ExitStack() as stack:
        for provider in PROVIDERS:
            scope = ("manifests", f"provider={provider}", "market=kr")
            lock = stack.enter_context(open_at(raw_fd, (*scope, "commit.lock")))
            fcntl.flock(lock, fcntl.LOCK_SH | fcntl.LOCK_NB)
        yield


def metadata_fingerprint(value):
    """Hash semantic JSON metadata without retaining every duplicate record."""
    try:
        wire = json.dumps(value, ensure_ascii=True, sort_keys=True, separators=(",", ":"),
                          allow_nan=False).encode("ascii")
    except (TypeError, ValueError, RecursionError):
        raise InventoryError("INVALID_MANIFEST") from None
    return hashlib.sha256(wire).digest()


def capacity_scope(provider, wire):
    """Count manifest identities without bodies; differing duplicates are indeterminate because
    this bounded inventory does not normalize every optional native metadata field.
    """
    require(len(wire) <= MANIFEST_LIMIT, "RAW_LIMIT_EXCEEDED")
    records = 0
    by_batch = {}
    dates = set()
    files = 0
    largest_batch_files = 0
    for raw_line in io.BytesIO(wire):
        if not raw_line.strip(b" \t\n\r\f"):
            continue
        records += 1
        require(records <= CAPACITY_RECORD_INSPECTION_LIMIT,
                "CAPACITY_INSPECTION_LIMIT")
        terminated = raw_line.endswith(b"\n")
        line = raw_line[:-1] if terminated else raw_line
        try:
            record = decode(line)
        except InventoryError:
            raise
        except json.JSONDecodeError:
            if not terminated:
                raise InventoryError("INDETERMINATE_MANIFEST_TAIL") from None
            raise InventoryError("INVALID_MANIFEST") from None
        except (ValueError, TypeError, RecursionError):
            raise InventoryError("INVALID_MANIFEST") from None
        require(isinstance(record, dict), "INVALID_MANIFEST")
        batch_id = record.get("batch_id")
        require(isinstance(batch_id, str), "INVALID_MANIFEST")
        try:
            parsed_id = uuid.UUID(batch_id)
            partition_date = iso_date(record.get("date"))
        except ValueError:
            raise InventoryError("INVALID_MANIFEST") from None
        require(str(parsed_id) == batch_id and parsed_id.int != 0, "INVALID_MANIFEST")
        require(record.get("provider") == provider and record.get("market") == "kr",
                "INVALID_MANIFEST")
        batch_files = record.get("files")
        require(isinstance(batch_files, list), "INVALID_MANIFEST")
        require(len(batch_files) <= CAPACITY_FILE_INSPECTION_LIMIT,
                "CAPACITY_INSPECTION_LIMIT")
        fingerprint = metadata_fingerprint(record)
        if batch_id in by_batch:
            require(by_batch[batch_id] == fingerprint, "INDETERMINATE_DUPLICATE_METADATA")
            continue
        require(files + len(batch_files) <= CAPACITY_FILE_INSPECTION_LIMIT,
                "CAPACITY_INSPECTION_LIMIT")
        names = set()
        for metadata in batch_files:
            require(isinstance(metadata, dict), "INVALID_MANIFEST")
            file_name = metadata.get("file_name")
            require(capacity_file_name(file_name), "INVALID_MANIFEST")
            require(file_name not in names, "INVALID_MANIFEST")
            names.add(file_name)
        by_batch[batch_id] = fingerprint
        dates.add(partition_date)
        files += len(names)
        largest_batch_files = max(largest_batch_files, len(names))
    batch_count = len(by_batch)
    live_descriptors = descriptor_projection(
        LIVE_DATA_ROOT_COMPONENTS, 0, batch_count, len(dates), files)
    archive_descriptors = descriptor_projection(
        ARCHIVE_DATA_ROOT_COMPONENTS, ARCHIVE_COORDINATION_COMPONENTS,
        batch_count, len(dates), files)
    return {
        "manifest_records": records,
        "batches": batch_count,
        "files": files,
        "largest_batch_files": largest_batch_files,
        "distinct_dates": len(dates),
        "manifest_bytes": len(wire),
        "live_descriptors_projected": live_descriptors,
        "paper_archive_descriptors_projected": archive_descriptors,
    }


def descriptor_projection(root_components, coordination_components, batches, dates, files):
    # Mirrors daily_price_only/source.rs. Empty and nonempty scopes have different bases.
    if batches == 0:
        return root_components + coordination_components + 7
    return root_components + coordination_components + 9 + dates + 2 * batches + files


def capacity_inventory(data_root):
    require(data_root.is_absolute() and data_root.anchor == "/", "INVALID_PATH")
    with ExitStack() as stack:
        system_root = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
        stack.callback(os.close, system_root)
        with open_at(system_root, (*data_root.parts[1:], "raw"), directory=True) as raw_fd:
            snapshots = []
            with shared_manifest_locks(raw_fd):
                for provider in PROVIDERS:
                    scope = ("manifests", f"provider={provider}", "market=kr")
                    wire = read_at(raw_fd, (*scope, "manifest.jsonl"), MANIFEST_LIMIT)
                    snapshots.append((provider, wire))
            scopes = []
            for provider, wire in snapshots:
                counts = capacity_scope(provider, wire)
                metrics = {
                    name: {
                        "observed": counts[name],
                        "inclusive_limit": limit,
                        "within_limit": counts[name] <= limit,
                    }
                    for name, limit in CAPACITY_LIMITS.items()
                }
                metrics["distinct_dates"] = counts["distinct_dates"]
                scopes.append({"provider": provider, "metrics": metrics})
    return {
        "status": "capacity_projection",
        "measurement": "committed_manifest_metadata_only",
        "descriptor_projection_assumptions": {
            "live_data_root_components": LIVE_DATA_ROOT_COMPONENTS,
            "paper_archive_data_root_components": ARCHIVE_DATA_ROOT_COMPONENTS,
            "paper_archive_coordination_components": ARCHIVE_COORDINATION_COMPONENTS,
        },
        "scopes": scopes,
        "not_assessed": [
            "provider body presence, size, or hash",
            "native full-schema or byte verification",
            "entitlement rights or runtime readiness",
        ],
    }


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
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--entitlement-sha256", help="configured entitlement SHA-256")
    mode.add_argument("--capacity-only", action="store_true",
                      help="report bounded committed-manifest capacity counts without entitlement or body reads")
    parser.add_argument("--through", help="inclusive calendar date, defaults to the 16:30 KST cutoff")
    try:
        args = parser.parse_args()
        if args.capacity_only:
            require(args.through is None, "INVALID_ARGUMENT")
            result = capacity_inventory(args.data_root)
        else:
            require(args.entitlement_sha256 is not None, "INVALID_ARGUMENT")
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
