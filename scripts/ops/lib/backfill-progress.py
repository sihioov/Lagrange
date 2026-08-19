#!/usr/bin/env python3
"""Relay safe worker output and durably record validated per-date progress."""

from __future__ import annotations

import datetime as dt
import json
import os
import re
import sys
import uuid


DEFERRED_EXIT = 75
RETRYABLE_EXIT = 74
SNAPSHOT_MISS_CODE = "KIS_CALENDAR_SNAPSHOT_MISS"
ERROR_CODE = re.compile(r"^[A-Z][A-Z0-9_]{0,63}$")
SAFE_PHASES = frozenset(
    {
        "config",
        "provider",
        "recovery",
        "duplicate_check",
        "ingest",
        "publication",
        "health",
        "database",
    }
)
SAFE_CLASSES = frozenset({"retryable", "permanent"})


def safe_date(value: object) -> str | None:
    if not isinstance(value, str):
        return None
    try:
        parsed = dt.date.fromisoformat(value)
    except ValueError:
        return None
    return parsed.isoformat() if parsed.isoformat() == value else None


def append_state(
    state_path: str,
    date_text: str,
    status: str,
    run_identity: str,
    error_code: str | None = None,
) -> None:
    fields = [date_text, status, run_identity]
    if error_code is not None:
        fields.append(error_code)
    with open(state_path, "a", encoding="ascii") as state:
        state.write("\t".join(fields) + "\n")
        state.flush()
        os.fsync(state.fileno())


def safe_error_line(
    error_code: str,
    date_text: str,
    phase: object,
    failure_class: object,
) -> str:
    """Return body-free, stable diagnostics for the operator stream."""

    record = {
        "status": "error",
        "error_code": error_code,
        "target_date": date_text,
        "phase": phase if isinstance(phase, str) and phase in SAFE_PHASES else "ingest",
        "class": (
            failure_class
            if isinstance(failure_class, str) and failure_class in SAFE_CLASSES
            else "permanent"
        ),
    }
    return json.dumps(record, separators=(",", ":"), sort_keys=True)


def record_failure(
    state_path: str,
    run_identity: str,
    expected: dt.date,
    record: dict,
) -> int:
    """Persist exactly the next affected date and return the wrapper exit code."""

    raw_code = record.get("error_code")
    error_code = raw_code if isinstance(raw_code, str) and ERROR_CODE.fullmatch(raw_code) else "WORKER_ERROR"
    record_date = safe_date(record.get("target_date"))
    expected_text = expected.isoformat()
    # BackfillRange has no command-level target_date.  Its worker always emits
    # dates in order, so a missing target is safely attributed to the next
    # expected date; a contradictory target is a protocol failure.
    if record_date is not None and record_date != expected_text:
        append_state(state_path, expected_text, "FAILED", run_identity, "BACKFILL_PROGRESS_INVALID")
        print(
            safe_error_line(
                "BACKFILL_PROGRESS_INVALID",
                expected_text,
                "publication",
                "permanent",
            ),
            flush=True,
        )
        return 1

    failure_class = record.get("class")
    if error_code == SNAPSHOT_MISS_CODE:
        status = "DEFERRED"
        exit_code = DEFERRED_EXIT
    elif failure_class == "retryable":
        status = "RETRYABLE"
        exit_code = RETRYABLE_EXIT
    else:
        # Every permanent/unknown error is a manual-review stop.  It must not
        # be retried by the recurring timer.
        status = "FAILED"
        exit_code = 1
    append_state(state_path, expected_text, status, run_identity, error_code)
    print(
        safe_error_line(
            error_code,
            expected_text,
            record.get("phase"),
            failure_class,
        ),
        flush=True,
    )
    return exit_code


def protocol_failure(state_path: str, run_identity: str, expected: dt.date, code: str) -> int:
    append_state(state_path, expected.isoformat(), "FAILED", run_identity, code)
    print(safe_error_line(code, expected.isoformat(), "publication", "permanent"), flush=True)
    return 1


def main() -> int:
    if len(sys.argv) != 5:
        raise SystemExit("backfill-progress: expected STATE IDENTITY START END")
    state_path, run_identity, start_text, end_text = sys.argv[1:]
    expected: dt.date | None = dt.date.fromisoformat(start_text)
    end = dt.date.fromisoformat(end_text)
    published: set[str] = set()
    with open(state_path, encoding="ascii") as existing:
        for line in existing:
            fields = line.rstrip("\n").split("\t")
            if len(fields) == 3 and fields[1:] == ["PUBLISHED", run_identity]:
                published.add(fields[0])

    expected_date = expected
    with open(state_path, "a", encoding="ascii") as state:
        for line in sys.stdin:
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                print(line, end="", flush=True)
                continue
            if isinstance(record, dict) and record.get("status") == "error":
                # Close the state file before the helper exits.  The error
                # record itself is rewritten to a stable, body-free shape by
                # record_failure; broker messages and endpoint details never
                # become part of the persisted or forwarded diagnostic.
                state.close()
                if expected_date is None:
                    print(
                        safe_error_line(
                            "BACKFILL_PROGRESS_INVALID",
                            start_text,
                            "publication",
                            "permanent",
                        ),
                        flush=True,
                    )
                    return 1
                return record_failure(state_path, run_identity, expected_date, record)
            print(line, end="", flush=True)
            if not (
                record.get("status") == "event"
                and record.get("event") == "published"
                and record.get("phase") == "canonical_publication"
            ):
                continue
            date_text = record.get("target_date")
            batch_id = record.get("batch_id")
            if expected_date is None or date_text != expected_date.isoformat() or not isinstance(batch_id, str):
                state.close()
                return protocol_failure(
                    state_path,
                    run_identity,
                    expected_date or end,
                    "BACKFILL_PROGRESS_INVALID",
                )
            try:
                uuid.UUID(batch_id)
            except ValueError as exc:
                del exc
                state.close()
                return protocol_failure(
                    state_path,
                    run_identity,
                    expected_date,
                    "BACKFILL_PROGRESS_INVALID",
                )
            if date_text not in published:
                state.write(f"{date_text}\tPUBLISHED\t{run_identity}\n")
                state.flush()
                os.fsync(state.fileno())
                published.add(date_text)
            print(f"BACKFILL_DONE date={date_text}", flush=True)
            expected_date = None if expected_date == end else expected_date + dt.timedelta(days=1)
    if expected_date is not None:
        return protocol_failure(state_path, run_identity, expected_date, "BACKFILL_INCOMPLETE")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
