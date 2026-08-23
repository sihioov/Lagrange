#!/usr/bin/env python3
"""Validate and durably initialize the KIS daily V4 progress state."""

from __future__ import annotations

import datetime as dt
import os
import re
import stat
import sys
from pathlib import Path


HEADER_NAME = "LAGRANGE_BACKFILL_STATE_V4"
IDENTITY = re.compile(r"^[0-9a-f]{64}$")
ERROR_CODE = re.compile(r"^[A-Z][A-Z0-9_]{0,63}$")
DIAGNOSTICS = {
    "DAILY_STATE_MISSING": 20,
    "DAILY_STATE_STALE": 21,
    "DAILY_STATE_MALFORMED": 22,
    "DAILY_STATE_NOT_APPENDABLE": 23,
}


class StateFailure(Exception):
    """An expected, body-free state failure."""

    def __init__(self, code: str) -> None:
        self.code = code


def fail(code: str) -> None:
    raise StateFailure(code)


def read_bytes(path: Path, *, missing_code: str) -> bytes:
    try:
        path_stat = os.lstat(path)
    except FileNotFoundError:
        fail(missing_code)
    except OSError:
        fail("DAILY_STATE_NOT_APPENDABLE" if missing_code == "DAILY_STATE_MISSING" else "DAILY_STATE_MALFORMED")
    if stat.S_ISLNK(path_stat.st_mode) or not stat.S_ISREG(path_stat.st_mode):
        fail("DAILY_STATE_NOT_APPENDABLE" if missing_code == "DAILY_STATE_MISSING" else "DAILY_STATE_MALFORMED")
    try:
        return path.read_bytes()
    except FileNotFoundError:
        fail(missing_code)
    except OSError:
        fail("DAILY_STATE_NOT_APPENDABLE" if missing_code == "DAILY_STATE_MISSING" else "DAILY_STATE_MALFORMED")


def decode_lines(raw: bytes) -> list[str]:
    try:
        text = raw.decode("ascii")
    except UnicodeDecodeError:
        fail("DAILY_STATE_MALFORMED")
    if not raw:
        return []
    if b"\r" in raw or not raw.endswith(b"\n"):
        fail("DAILY_STATE_MALFORMED")
    return text[:-1].split("\n")


def parse_date(value: object) -> dt.date | None:
    if not isinstance(value, str) or not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", value):
        return None
    try:
        parsed = dt.date.fromisoformat(value)
    except ValueError:
        return None
    return parsed if parsed.isoformat() == value else None


def parse_arguments(argv: list[str]) -> tuple[Path, str, Path, dt.date, dt.date]:
    if len(argv) != 5:
        fail("DAILY_STATE_MALFORMED")
    state_path = Path(argv[0])
    identity = argv[1]
    db_path = Path(argv[2])
    start = parse_date(argv[3])
    end = parse_date(argv[4])
    if not IDENTITY.fullmatch(identity) or start is None or end is None or end < start:
        fail("DAILY_STATE_MALFORMED")
    return state_path, identity, db_path, start, end


def validate_state(
    lines: list[str], identity: str, start: dt.date, end: dt.date
) -> set[str]:
    if not lines:
        return set()
    header = lines[0].split("\t")
    if len(header) != 2 or header[0] != HEADER_NAME:
        fail("DAILY_STATE_MALFORMED")
    stored_identity = header[1]
    if not IDENTITY.fullmatch(stored_identity):
        fail("DAILY_STATE_MALFORMED")
    if stored_identity != identity:
        fail("DAILY_STATE_STALE")

    published: set[str] = set()
    last_status: dict[str, str] = {}
    for fields in (line.split("\t") for line in lines[1:]):
        if len(fields) not in (3, 4):
            fail("DAILY_STATE_MALFORMED")
        date_text, status, record_identity = fields[:3]
        parsed_date = parse_date(date_text)
        if parsed_date is None or not start <= parsed_date <= end:
            fail("DAILY_STATE_MALFORMED")
        if not IDENTITY.fullmatch(record_identity):
            fail("DAILY_STATE_MALFORMED")
        if record_identity != identity:
            fail("DAILY_STATE_STALE")
        if status in {"RUNNING", "PUBLISHED"}:
            if len(fields) != 3:
                fail("DAILY_STATE_MALFORMED")
        elif status in {"FAILED", "DEFERRED", "RETRYABLE"}:
            if len(fields) != 4 or not ERROR_CODE.fullmatch(fields[3]):
                fail("DAILY_STATE_MALFORMED")
        else:
            fail("DAILY_STATE_MALFORMED")
        if last_status.get(date_text) == "PUBLISHED" and status != "PUBLISHED":
            fail("DAILY_STATE_MALFORMED")
        if status == "PUBLISHED":
            published.add(date_text)
        last_status[date_text] = status
    return published


def parse_db_dates(raw: bytes, start: dt.date, end: dt.date) -> list[str]:
    lines = decode_lines(raw)
    if not lines:
        fail("DAILY_STATE_MALFORMED")
    metadata: list[str] | None = None
    dates: list[str] = []
    for index, line in enumerate(lines):
        fields = line.split("\t")
        if len(fields) != 5:
            fail("DAILY_STATE_MALFORMED")
        if fields[0] == "META":
            if index != 0 or metadata is not None or dates:
                fail("DAILY_STATE_MALFORMED")
            metadata = fields[1:]
            continue
        if fields[0] != "DATE" or metadata is None or fields[3:] != ["-", "-"]:
            fail("DAILY_STATE_MALFORMED")
        date_text, count_text = fields[1], fields[2]
        parsed_date = parse_date(date_text)
        if parsed_date is None or not start <= parsed_date <= end or count_text != "1":
            fail("DAILY_STATE_MALFORMED")
        dates.append(date_text)
    if metadata is None or not dates or dates != sorted(dates) or len(dates) != len(set(dates)):
        fail("DAILY_STATE_MALFORMED")
    first_text, last_text, date_count_text, row_count_text = metadata
    if first_text != dates[0] or last_text != dates[-1]:
        fail("DAILY_STATE_MALFORMED")
    if date_count_text != str(len(dates)) or row_count_text != str(len(dates)):
        fail("DAILY_STATE_MALFORMED")
    if first_text != start.isoformat():
        fail("DAILY_STATE_MALFORMED")
    return dates


def append_state(path: Path, payload: bytes) -> None:
    flags = os.O_WRONLY | os.O_APPEND | getattr(os, "O_NOFOLLOW", 0)
    try:
        fd = os.open(path, flags)
        with os.fdopen(fd, "ab", closefd=True) as state:
            if payload:
                state.write(payload)
                state.flush()
            os.fsync(state.fileno())
    except FileNotFoundError:
        fail("DAILY_STATE_MISSING")
    except OSError:
        fail("DAILY_STATE_NOT_APPENDABLE")
    except Exception:
        fail("DAILY_STATE_NOT_APPENDABLE")


def run(argv: list[str]) -> None:
    state_path, identity, db_path, start, end = parse_arguments(argv)
    state_lines = decode_lines(read_bytes(state_path, missing_code="DAILY_STATE_MISSING"))
    published = validate_state(state_lines, identity, start, end)
    db_dates = parse_db_dates(
        read_bytes(db_path, missing_code="DAILY_STATE_MALFORMED"), start, end
    )
    to_append = [date_text for date_text in db_dates if date_text not in published]
    payload = b""
    if not state_lines:
        payload += f"{HEADER_NAME}\t{identity}\n".encode("ascii")
    payload += "".join(
        f"{date_text}\tPUBLISHED\t{identity}\n" for date_text in to_append
    ).encode("ascii")
    append_state(state_path, payload)


def main(argv: list[str]) -> int:
    try:
        run(argv)
    except StateFailure as failure:
        sys.stderr.write(failure.code + "\n")
        return DIAGNOSTICS[failure.code]
    except Exception:
        sys.stderr.write("DAILY_STATE_MALFORMED\n")
        return DIAGNOSTICS["DAILY_STATE_MALFORMED"]
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
