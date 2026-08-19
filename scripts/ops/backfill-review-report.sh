#!/usr/bin/env bash
# Local-only, non-approving report for a completed read-only ETF backfill.
#
# This is deliberately separate from the DB/worker health gate and from READY
# dataset registration.  It reads only the durable backfill state, Raw
# manifest indexes, and curated manifest/artifact bytes.  It never changes a
# file and never talks to an external service.
set -euo pipefail

mode=plan
state_file=${LAGRANGE_BACKFILL_STATE:-}
data_root=${LAGRANGE_DATA_DIR:-/var/lib/lagrange/data}
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
calendar_dir=${LAGRANGE_XKRX_CALENDAR_DIR:-$repo_root/data/calendars/xkrx}
start_date=
end_date=
dataset_id=krx_eod_bars

usage() {
  cat <<'USAGE'
Usage: scripts/ops/backfill-review-report.sh --start YYYY-MM-DD --end YYYY-MM-DD
       [--state-file PATH] [--data-root ABSOLUTE_PATH]
       [--dataset-id ID] [--plan|--check]

  --plan   Print the local review contract without reading production data.
  --check  Read the state/Raw/Curated files and emit a non-approving report.

The report never registers READY, writes pins, or asserts DB/entitlement
approval.  A complete report is only an operator review handoff.
USAGE
}

die() { echo "backfill-review-report: $*" >&2; exit 1; }
while [ "$#" -gt 0 ]; do
  case "$1" in
    --plan) mode=plan; shift ;;
    --check) mode=check; shift ;;
    --state-file) [ "$#" -ge 2 ] || die '--state-file needs a path'; state_file=$2; shift 2 ;;
    --data-root) [ "$#" -ge 2 ] || die '--data-root needs a path'; data_root=$2; shift 2 ;;
    --dataset-id) [ "$#" -ge 2 ] || die '--dataset-id needs a value'; dataset_id=$2; shift 2 ;;
    --start) [ "$#" -ge 2 ] || die '--start needs a date'; start_date=$2; shift 2 ;;
    --end) [ "$#" -ge 2 ] || die '--end needs a date'; end_date=$2; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

[ -n "$start_date" ] || die '--start is required'
[ -n "$end_date" ] || die '--end is required'
[ -n "$state_file" ] || die '--state-file is required (or set LAGRANGE_BACKFILL_STATE)'

if [ "$mode" = plan ]; then
  cat <<EOF
BACKFILL_REVIEW_PLAN: local-only non-approving ETF report
  range: $start_date..$end_date
  state: $state_file
  data_root: $data_root
  dataset: $dataset_id
  checks: V4 state completion, validated XKRX session-date Raw coverage, curated manifest/artifact integrity
  decision: report only; READY registration, DB readiness, entitlement approval, and release pins remain separate
PLAN_ONLY: no production file read, write, or external service action made
EOF
  exit 0
fi

command -v python3 >/dev/null 2>&1 || die 'python3 is required for --check'
exec python3 - "$state_file" "$start_date" "$end_date" "$data_root" "$dataset_id" "$calendar_dir" "$repo_root" <<'PY'
from __future__ import annotations

import datetime as dt
import hashlib
import json
import os
import re
import stat
import subprocess
import sys
from pathlib import Path


class ReportBlocked(Exception):
    pass


def fail(message: str) -> None:
    raise ReportBlocked(message)


def safe_existing_path(path: Path, label: str, want_dir: bool | None = None) -> None:
    if not path.is_absolute() or ".." in path.parts:
        fail(f"{label} must be absolute and must not contain ..")
    current = Path(path.anchor)
    for part in path.parts[1:]:
        current /= part
        if not current.exists():
            fail(f"{label} component is missing: {current}")
        metadata = os.lstat(current)
        if stat.S_ISLNK(metadata.st_mode):
            fail(f"{label} contains a symlink: {current}")
    metadata = os.lstat(path)
    if stat.S_ISLNK(metadata.st_mode):
        fail(f"{label} must not be a symlink")
    if want_dir is True and not stat.S_ISDIR(metadata.st_mode):
        fail(f"{label} must be a directory")
    if want_dir is False and not stat.S_ISREG(metadata.st_mode):
        fail(f"{label} must be a regular file")


def parse_date(value: str, label: str) -> dt.date:
    try:
        parsed = dt.date.fromisoformat(value)
    except ValueError as exc:
        fail(f"{label} is not a real YYYY-MM-DD date")
        raise AssertionError from exc
    if parsed.isoformat() != value:
        fail(f"{label} is not canonical YYYY-MM-DD")
    return parsed


def validated_session_dates(
    start: dt.date, end: dt.date, calendar_dir_text: str, repo_root_text: str
) -> list[str]:
    calendar_dir = Path(calendar_dir_text)
    safe_existing_path(calendar_dir, "XKRX calendar directory", want_dir=True)
    script = Path(repo_root_text) / "scripts" / "ops" / "xkrx-calendar-bootstrap.py"
    safe_existing_path(script, "XKRX calendar bootstrap", want_dir=False)
    result = subprocess.run(
        [
            sys.executable,
            str(script),
            "--emit-sessions",
            "--start",
            start.isoformat(),
            "--end",
            end.isoformat(),
            "--output-dir",
            str(calendar_dir),
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        fail("XKRX scheduler artifact validation failed")
    metadata_lines = result.stderr.splitlines()
    if len(metadata_lines) != 1:
        fail("XKRX scheduler metadata is malformed")
    try:
        metadata = json.loads(metadata_lines[0])
    except json.JSONDecodeError:
        fail("XKRX scheduler metadata is not JSON")
    if not isinstance(metadata, dict) or metadata.get("requested_range") != {
        "start": start.isoformat(),
        "end": end.isoformat(),
    }:
        fail("XKRX scheduler metadata range mismatch")
    dates = result.stdout.splitlines()
    if dates != sorted(dates) or len(dates) != len(set(dates)):
        fail("XKRX scheduler session dates are not sorted and unique")
    return dates


def canonical_manifest_hash(value: dict) -> str:
    fields = {
        "dataset_id": value["dataset_id"],
        "version": value["version"],
        "capability": value["capability"],
        "created_at": value["created_at"],
        "source_batches": value["source_batches"],
        "artifacts": value["artifacts"],
        "bar_count": value["bar_count"],
        "action_count": value["action_count"],
    }
    encoded = json.dumps(fields, ensure_ascii=False, separators=(",", ":"))
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def safe_artifact(curated_root: Path, artifact: dict) -> tuple[str, str | None]:
    path = artifact.get("path")
    if (
        not isinstance(path, str)
        or not path
        or path.startswith("/")
        or "\\" in path
        or any(part in ("", ".", "..") for part in path.split("/"))
    ):
        return "", "unsafe artifact path"
    digest = artifact.get("sha256")
    size = artifact.get("size_bytes")
    schema = artifact.get("schema")
    if (
        not isinstance(digest, str)
        or not digest.startswith("sha256:")
        or len(digest) != 71
        or any(char not in "0123456789abcdef" for char in digest[7:])
        or not isinstance(size, int)
        or size < 0
        or not isinstance(schema, str)
        or schema
        not in {
            "bars-v1",
            "adjusted-bars-v1",
            "total-return-bars-v1",
            "corporate-actions-v2",
        }
    ):
        return path, "invalid artifact reference"
    target = curated_root.joinpath(*path.split("/"))
    try:
        safe_existing_path(target, f"curated artifact {path}", want_dir=False)
    except ReportBlocked as exc:
        return path, str(exc)
    if target.resolve() != target:
        return path, "artifact resolves outside its declared path"
    raw = target.read_bytes()
    if len(raw) != size:
        return path, "artifact size differs from manifest"
    if hashlib.sha256(raw).hexdigest() != digest[7:]:
        return path, "artifact SHA-256 differs from manifest"
    if len(raw) < 8 or raw[:4] != b"PAR1" or raw[-4:] != b"PAR1":
        return path, "artifact is not a complete Parquet file"
    return path, None


def read_raw_manifest(data_root: Path, provider: str, expected: set[str]) -> tuple[int, set[str]]:
    path = data_root / "raw" / "manifests" / f"provider={provider}" / "market=kr" / "manifest.jsonl"
    safe_existing_path(path, f"Raw {provider}/kr manifest", want_dir=False)
    rows = 0
    dates: set[str] = set()
    with path.open("r", encoding="utf-8") as stream:
        for line_number, line in enumerate(stream, 1):
            if not line.strip():
                continue
            try:
                row = json.loads(line)
            except json.JSONDecodeError as exc:
                fail(f"Raw {provider}/kr manifest line {line_number} is malformed: {exc}")
            if not isinstance(row, dict):
                fail(f"Raw {provider}/kr manifest line {line_number} is not an object")
            if row.get("provider") != provider or row.get("market") != "kr":
                fail(f"Raw {provider}/kr manifest line {line_number} has a scope mismatch")
            date = row.get("date")
            if not isinstance(date, str):
                fail(f"Raw {provider}/kr manifest line {line_number} has no date")
            rows += 1
            if date in expected:
                dates.add(date)
    return rows, dates


def curated_candidates(data_root: Path, dataset_id: str) -> tuple[list[dict], list[str]]:
    curated_root = data_root / "curated"
    safe_existing_path(curated_root, "curated root", want_dir=True)
    dataset_root = curated_root / "datasets" / dataset_id
    if not dataset_root.exists():
        return [], ["curated dataset directory is missing"]
    safe_existing_path(dataset_root, "curated dataset directory", want_dir=True)
    candidates: list[dict] = []
    issues: list[str] = []
    for version_dir in sorted(dataset_root.iterdir(), key=lambda item: item.name):
        if not version_dir.name.startswith("version="):
            continue
        if version_dir.is_symlink() or not version_dir.is_dir():
            issues.append(f"{version_dir.name}: version directory is not a regular directory")
            continue
        version_text = version_dir.name.removeprefix("version=")
        if not version_text.isdigit() or int(version_text) < 1:
            issues.append(f"{version_dir.name}: invalid version")
            continue
        manifest_path = version_dir / "manifest.json"
        try:
            safe_existing_path(manifest_path, f"{version_dir.name} manifest", want_dir=False)
            value = json.loads(manifest_path.read_text(encoding="utf-8"))
        except (ReportBlocked, OSError, json.JSONDecodeError) as exc:
            issues.append(f"{version_dir.name}: malformed or unreadable manifest ({exc})")
            continue
        if not isinstance(value, dict):
            issues.append(f"{version_dir.name}: manifest is not an object")
            continue
        required = ("dataset_id", "version", "content_hash", "source_batches", "artifacts")
        if any(key not in value for key in required):
            issues.append(f"{version_dir.name}: manifest is missing required fields")
            continue
        if value["dataset_id"] != dataset_id or value["version"] != int(version_text):
            issues.append(f"{version_dir.name}: manifest identity does not match its path")
            continue
        content_hash = value["content_hash"]
        if (
            not isinstance(content_hash, str)
            or not content_hash.startswith("sha256:")
            or len(content_hash) != 71
            or content_hash[7:] != canonical_manifest_hash(value)
        ):
            issues.append(f"{version_dir.name}: canonical manifest hash mismatch")
            continue
        if not isinstance(value["source_batches"], list) or not value["source_batches"]:
            issues.append(f"{version_dir.name}: source batch inventory is empty")
            continue
        if not isinstance(value["artifacts"], list) or not value["artifacts"]:
            issues.append(f"{version_dir.name}: exact artifact inventory is empty")
            continue
        artifact_errors = []
        for artifact in value["artifacts"]:
            if not isinstance(artifact, dict):
                artifact_errors.append("artifact entry is not an object")
                continue
            _, error = safe_artifact(curated_root, artifact)
            if error:
                artifact_errors.append(error)
        if artifact_errors:
            issues.append(f"{version_dir.name}: {artifact_errors[0]}")
            continue
        candidates.append(
            {
                "version": int(version_text),
                "manifest_sha256": content_hash[7:],
                "artifacts": len(value["artifacts"]),
                "bar_count": value.get("bar_count", "unknown"),
                "action_count": value.get("action_count", "unknown"),
            }
        )
    return candidates, issues


def main() -> int:
    state_file, start_text, end_text, data_root_text, dataset_id, calendar_dir_text, repo_root_text = sys.argv[1:]
    start = parse_date(start_text, "start date")
    end = parse_date(end_text, "end date")
    if end < start:
        fail("end date precedes start date")
    expected_dates = validated_session_dates(start, end, calendar_dir_text, repo_root_text)
    expected = set(expected_dates)
    if len(expected_dates) > 10000:
        fail("date range is unreasonably large")

    state = Path(state_file)
    data_root = Path(data_root_text)
    safe_existing_path(state, "backfill state", want_dir=False)
    safe_existing_path(data_root, "data root", want_dir=True)
    lines = state.read_text(encoding="ascii").splitlines()
    if not lines or len(lines[0].split("\t")) != 2 or lines[0].split("\t")[0] != "LAGRANGE_BACKFILL_STATE_V4":
        fail("backfill state is not a V4 file")
    run_identity = lines[0].split("\t", 1)[1]
    if len(run_identity) != 64 or any(char not in "0123456789abcdef" for char in run_identity):
        fail("backfill state identity is malformed")
    latest: dict[str, str] = {}
    history_failures = 0
    history_deferred = 0
    history_retryable = 0
    error_code_pattern = re.compile(r"^[A-Z][A-Z0-9_]{0,63}$")
    for line_number, line in enumerate(lines[1:], 2):
        fields = line.split("\t")
        if len(fields) not in (3, 4):
            fail(f"backfill state line {line_number} has an invalid field count")
        date, status, identity = fields[:3]
        if date not in expected or identity != run_identity:
            fail(f"backfill state line {line_number} is outside the requested run")
        if status in {"RUNNING", "PUBLISHED"}:
            if len(fields) != 3:
                fail(f"backfill state line {line_number} has an unexpected error code")
        elif status in {"DEFERRED", "RETRYABLE", "FAILED"}:
            if len(fields) != 4 or not error_code_pattern.fullmatch(fields[3]):
                fail(f"backfill state line {line_number} has an invalid error code")
        else:
            fail(f"backfill state line {line_number} has an invalid status")
        if status == "FAILED":
            history_failures += 1
        elif status == "DEFERRED":
            history_deferred += 1
        elif status == "RETRYABLE":
            history_retryable += 1
        latest[date] = status
    missing_state = [date for date in expected_dates if latest.get(date) != "PUBLISHED"]
    published = len(expected_dates) - len(missing_state)

    raw_summary = {}
    raw_missing = []
    for provider in ("kis", "kis-normalized"):
        rows, dates = read_raw_manifest(data_root, provider, expected)
        raw_summary[provider] = (rows, len(dates))
        raw_missing.extend(f"{provider}:{date}" for date in expected_dates if date not in dates)

    candidates, curated_issues = curated_candidates(data_root, dataset_id)
    print("BACKFILL_REVIEW_REPORT: local-only non-approving")
    print(f"  range={start_text}..{end_text} expected_dates={len(expected_dates)} run_identity={run_identity}")
    print(
        f"  state published={published}/{len(expected_dates)} "
        f"latest_incomplete={len(missing_state)} historical_failures={history_failures} "
        f"historical_deferred={history_deferred} historical_retryable={history_retryable}"
    )
    for provider in ("kis", "kis-normalized"):
        rows, dates = raw_summary[provider]
        print(f"  raw provider={provider} market=kr manifest_rows={rows} covered_dates={dates}/{len(expected_dates)}")
    print(f"  curated dataset={dataset_id} attested_candidates={len(candidates)} malformed_or_incomplete={len(curated_issues)}")
    for candidate in sorted(candidates, key=lambda item: item["version"]):
        print(
            "  candidate version={version} artifacts={artifacts} bars={bar_count} actions={action_count} manifest_sha256={manifest_sha256}".format(**candidate)
        )
    if missing_state:
        print(f"  missing_or_nonterminal_dates={','.join(missing_state[:12])}{'...' if len(missing_state) > 12 else ''}")
    if raw_missing:
        print(f"  raw_missing_dates={','.join(raw_missing[:12])}{'...' if len(raw_missing) > 12 else ''}")
    if curated_issues:
        print(f"  curated_issues={curated_issues[0]}")
    print("  DB_READY=NOT_CHECKED dataset_pins=NOT_CHECKED entitlement_approval=NOT_CHECKED")
    if missing_state or raw_missing:
        print("BACKFILL_REVIEW: BLOCKED (backfill state or Raw coverage is incomplete)")
        return 2
    if not candidates:
        print("BACKFILL_REVIEW: WAITING_FOR_CURATED_OUTPUT (no attested candidate; no approval implied)")
        return 2
    print("BACKFILL_REVIEW: CURATED_CANDIDATE_FOUND_UNAPPROVED (operator review and READY registration remain required)")
    return 0


try:
    raise SystemExit(main())
except ReportBlocked as exc:
    print(f"BACKFILL_REVIEW: BLOCKED ({exc})", file=sys.stderr)
    raise SystemExit(2)
PY
