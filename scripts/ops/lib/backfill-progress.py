#!/usr/bin/env python3
"""Relay safe worker output and durably record validated per-date progress."""

from __future__ import annotations

import datetime as dt
import json
import os
import sys
import uuid


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

    with open(state_path, "a", encoding="ascii") as state:
        for line in sys.stdin:
            print(line, end="", flush=True)
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                continue
            if not (
                record.get("status") == "event"
                and record.get("event") == "published"
                and record.get("phase") == "canonical_publication"
            ):
                continue
            date_text = record.get("target_date")
            batch_id = record.get("batch_id")
            if expected is None or date_text != expected.isoformat() or not isinstance(batch_id, str):
                raise SystemExit("backfill-progress: worker progress is out of order or malformed")
            try:
                uuid.UUID(batch_id)
            except ValueError as exc:
                raise SystemExit("backfill-progress: worker progress batch id is malformed") from exc
            if date_text not in published:
                state.write(f"{date_text}\tPUBLISHED\t{run_identity}\n")
                state.flush()
                os.fsync(state.fileno())
                published.add(date_text)
            print(f"BACKFILL_DONE date={date_text}", flush=True)
            expected = None if expected == end else expected + dt.timedelta(days=1)
    if expected is not None:
        raise SystemExit("backfill-progress: worker ended before every date reported publication")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
