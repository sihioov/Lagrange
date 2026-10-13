#!/usr/bin/env python3
"""Write bounded CI cache evidence and compare a cold/warm pair."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import re
import stat
import sys
from pathlib import Path
from typing import Any


SCHEMA_VERSION = 1
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
NAMESPACE_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
KEY_RE = re.compile(r"^[A-Za-z0-9._-]{1,512}$")
DURATION_STAGES = ("install", "build", "test")


class EvidenceError(ValueError):
    """Malformed or incomparable cache evidence."""


def validate_sha(value: Any) -> str:
    if not isinstance(value, str) or not SHA_RE.fullmatch(value):
        raise EvidenceError("a complete 40-character lowercase commit SHA is required")
    return value


def validate_namespace(value: Any) -> str:
    if not isinstance(value, str) or not NAMESPACE_RE.fullmatch(value):
        raise EvidenceError("cache namespace must be 1-64 ASCII letters, digits, dot, underscore, or hyphen")
    return value


def restore_state(
    *, hit: str, matched_key: str, primary_key: str, outcome: str
) -> str:
    if outcome != "success":
        return "unknown"
    if hit == "true":
        if matched_key != primary_key:
            return "unknown"
        return "exact"
    if hit == "false" and matched_key:
        return "prefix" if matched_key != primary_key else "unknown"
    if hit in {"", "false"} and not matched_key:
        return "miss"
    return "unknown"


def save_state(outcome: str) -> str:
    if outcome == "failure":
        return "failed"
    if outcome == "skipped":
        return "not_attempted"
    if outcome == "success":
        # actions/cache/save can warn and still exit successfully when its
        # write token is read-only. A later exact restore is the save proof.
        return "attempted_unverified"
    return "unknown"


def parse_cache_entry(raw: str) -> dict[str, Any]:
    fields = raw.split("|")
    if len(fields) != 7:
        raise EvidenceError("cache entry must have seven pipe-separated fields")
    kind, primary_key, matched_key, hit, restore_outcome, save_outcome, path_bytes = fields
    if not re.fullmatch(r"[a-z][a-z0-9-]{0,63}", kind):
        raise EvidenceError("cache kind is malformed")
    if not KEY_RE.fullmatch(primary_key):
        raise EvidenceError("primary cache key is malformed")
    if matched_key and not KEY_RE.fullmatch(matched_key):
        raise EvidenceError("matched cache key is malformed")
    if hit not in {"", "true", "false"}:
        raise EvidenceError("cache-hit output is malformed")
    if restore_outcome not in {"success", "failure", "skipped", "cancelled", "unknown"}:
        raise EvidenceError("cache restore outcome is malformed")
    if save_outcome not in {"success", "failure", "skipped", "cancelled", "unknown"}:
        raise EvidenceError("cache save outcome is malformed")
    if path_bytes != "unavailable" and (len(path_bytes) > 20 or not path_bytes.isascii() or not path_bytes.isdecimal()):
        raise EvidenceError("cache path byte count must be a non-negative integer or unavailable")
    return {
        "kind": kind,
        "key": primary_key,
        "restore": restore_state(
            hit=hit,
            matched_key=matched_key,
            primary_key=primary_key,
            outcome=restore_outcome,
        ),
        "matched_key": matched_key or None,
        "save": save_state(save_outcome),
        "bytes": {"path": int(path_bytes) if path_bytes != "unavailable" else None, "archive": None},
    }


def parse_duration(raw: str) -> tuple[str, int | None]:
    stage, separator, value = raw.partition("=")
    if not separator or stage not in DURATION_STAGES:
        raise EvidenceError("duration must be install=<seconds>, build=<seconds>, or test=<seconds>")
    if value == "unavailable":
        return stage, None
    if len(value) > 10 or not value.isascii() or not value.isdecimal():
        raise EvidenceError("duration must be a non-negative integer or unavailable")
    return stage, int(value)


def duration_totals(values: list[str]) -> dict[str, Any]:
    grouped: dict[str, list[int | None]] = {stage: [] for stage in DURATION_STAGES}
    for raw in values:
        stage, seconds = parse_duration(raw)
        grouped[stage].append(seconds)
    totals: dict[str, int | None] = {}
    measured: dict[str, int] = {}
    for stage in DURATION_STAGES:
        items = grouped[stage]
        measured[stage] = sum(value is not None for value in items)
        totals[stage] = sum(value for value in items if value is not None) if items and all(value is not None for value in items) else None
    total = sum(value for value in totals.values() if value is not None) if all(value is not None for value in totals.values()) else None
    return {"install": totals["install"], "build": totals["build"], "test": totals["test"], "total": total, "measured_steps": measured}


def validate_record(value: Any, *, source: str) -> dict[str, Any]:
    if not isinstance(value, dict) or type(value.get("schema_version")) is not int or value.get("schema_version") != SCHEMA_VERSION:
        raise EvidenceError(f"{source} has an unsupported or malformed schema")
    validate_sha(value.get("sha"))
    validate_namespace(value.get("namespace"))
    for name in ("workflow", "job"):
        if (
            not isinstance(value.get(name), str)
            or not re.fullmatch(r"[A-Za-z0-9 ._/-]{1,128}", value[name])
        ):
            raise EvidenceError(f"{source} is missing {name}")
    caches = value.get("caches")
    if not isinstance(caches, list) or not 1 <= len(caches) <= 8:
        raise EvidenceError(f"{source} is missing cache entries")
    seen: set[str] = set()
    for item in caches:
        if not isinstance(item, dict) or not isinstance(item.get("kind"), str) or not re.fullmatch(r"[a-z][a-z0-9-]{0,63}", item["kind"]):
            raise EvidenceError(f"{source} has a malformed cache entry")
        if item["kind"] in seen:
            raise EvidenceError(f"{source} has duplicate cache kinds")
        seen.add(item["kind"])
        if not isinstance(item.get("key"), str) or not KEY_RE.fullmatch(item["key"]):
            raise EvidenceError(f"{source} has a malformed cache key")
        if item.get("restore") not in {"exact", "prefix", "miss", "unknown"}:
            raise EvidenceError(f"{source} has a malformed restore state")
        matched = item.get("matched_key")
        if matched is not None and (not isinstance(matched, str) or not KEY_RE.fullmatch(matched)):
            raise EvidenceError(f"{source} has a malformed matched cache key")
        if (
            (item["restore"] == "exact" and matched != item["key"])
            or (item["restore"] == "prefix" and (not matched or matched == item["key"]))
            or (item["restore"] == "miss" and matched is not None)
        ):
            raise EvidenceError(f"{source} has inconsistent cache match observations")
        if item.get("save") not in {"attempted_unverified", "failed", "not_attempted", "unknown"}:
            raise EvidenceError(f"{source} has a malformed save state")
        amounts = item.get("bytes")
        if not isinstance(amounts, dict) or set(amounts) != {"path", "archive"}:
            raise EvidenceError(f"{source} is missing cache byte observations")
        if any(amount is not None and (type(amount) is not int or amount < 0) for amount in amounts.values()):
            raise EvidenceError(f"{source} has malformed cache byte observations")
    durations = value.get("durations_seconds")
    if not isinstance(durations, dict):
        raise EvidenceError(f"{source} is missing duration totals")
    for field in ("install", "build", "test", "total"):
        if field not in durations:
            raise EvidenceError(f"{source} is missing {field} duration")
        amount = durations.get(field)
        if amount is not None and (type(amount) is not int or amount < 0):
            raise EvidenceError(f"{source} has a malformed {field} duration")
    stages = [durations[field] for field in DURATION_STAGES]
    expected_total = sum(stages) if all(amount is not None for amount in stages) else None
    if durations["total"] != expected_total:
        raise EvidenceError(f"{source} has inconsistent duration totals")
    return value


def _private_json(path: Path, value: dict[str, Any]) -> None:
    parent = path.parent
    parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    parent_info = parent.lstat()
    if not stat.S_ISDIR(parent_info.st_mode) or parent_info.st_uid != os.geteuid() or stat.S_IMODE(parent_info.st_mode) != 0o700:
        raise EvidenceError("cache evidence output directory must be owner-private")
    if path.exists() and (path.is_symlink() or not stat.S_ISREG(path.lstat().st_mode)):
        raise EvidenceError("cache evidence output must be a regular file")
    flags = os.O_WRONLY | os.O_CREAT | os.O_TRUNC | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags, 0o600)
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            descriptor = -1
            json.dump(value, stream, ensure_ascii=False, sort_keys=True, indent=2)
            stream.write("\n")
    finally:
        if descriptor >= 0:
            os.close(descriptor)


def _append_summary(path: Path, title: str, value: dict[str, Any]) -> None:
    if path.is_symlink():
        raise EvidenceError("GitHub step summary path must not be a symlink")
    durations = value.get("durations_seconds", {})
    summary = [
        f"### {title}",
        "",
        f"Workflow/job: `{value.get('workflow', 'comparison')}` / `{value.get('job', 'cold-warm')}`",
        f"SHA: `{value['sha']}`; cache namespace: `{value['namespace']}`",
        "",
    ]
    if "cold" in durations:
        summary.extend(
            [
                "| Cache | Cold key | Warm key | Cold → warm restore | Cold → warm save | Cold / warm path bytes |",
                "|---|---|---|---|---|---:|",
            ]
        )
        for item in value["caches"]:
            cold_bytes = item["cold_bytes"]["path"]
            warm_bytes = item["warm_bytes"]["path"]
            summary.append(
                f"| `{item['kind']}` | `{item['cold_key']}` | `{item['warm_key']}` | "
                f"{item['cold_restore']} → {item['warm_restore']} | "
                f"{item['cold_save']} → {item['warm_save']} | "
                f"{cold_bytes if cold_bytes is not None else 'unavailable'} / "
                f"{warm_bytes if warm_bytes is not None else 'unavailable'} |"
            )
        for phase in ("cold", "warm"):
            phase_durations = durations[phase]
            summary.extend(
                [
                    "",
                    f"{phase.title()} durations (seconds): install `{phase_durations.get('install') if phase_durations.get('install') is not None else 'unavailable'}`, "
                    f"build `{phase_durations.get('build') if phase_durations.get('build') is not None else 'unavailable'}`, "
                    f"test `{phase_durations.get('test') if phase_durations.get('test') is not None else 'unavailable'}`, "
                    f"total `{phase_durations.get('total') if phase_durations.get('total') is not None else 'unavailable'}`.",
                ]
            )
        delta = durations.get("delta_total")
        summary.extend(["", f"Warm minus cold total seconds: `{delta if delta is not None else 'unavailable'}`"])
    else:
        summary.extend(
            [
                "| Cache | Key | Restore | Save | Path bytes | Archive bytes |",
                "|---|---|---|---|---:|---:|",
            ]
        )
        for item in value["caches"]:
            path_bytes = item["bytes"]["path"]
            summary.append(
                f"| `{item['kind']}` | `{item['key']}` | `{item['restore']}` | `{item['save']}` | "
                f"{path_bytes if path_bytes is not None else 'unavailable'} | unavailable |"
            )
        summary.extend(
            [
                "",
                "| Install seconds | Build seconds | Test seconds | Total seconds |",
                "|---:|---:|---:|---:|",
                "| " + " | ".join(str(durations.get(name) if durations.get(name) is not None else "unavailable") for name in ("install", "build", "test", "total")) + " |",
            ]
        )
    summary.extend(["", "```json", json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2), "```", ""])
    flags = os.O_WRONLY | os.O_APPEND | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags, 0o600)
    try:
        with os.fdopen(descriptor, "a", encoding="utf-8") as stream:
            descriptor = -1
            stream.write("\n".join(summary))
    finally:
        if descriptor >= 0:
            os.close(descriptor)


def record(args: argparse.Namespace) -> dict[str, Any]:
    sha = validate_sha(args.sha)
    namespace = validate_namespace(args.namespace)
    caches = [parse_cache_entry(item) for item in args.cache_entry]
    if len({item["kind"] for item in caches}) != len(caches):
        raise EvidenceError("cache kinds must be unique in one job record")
    if len(args.duration) > 64:
        raise EvidenceError("at most 64 command duration observations are allowed")
    durations = duration_totals(args.duration)
    report = {
        "schema_version": SCHEMA_VERSION,
        "workflow": args.workflow,
        "job": args.job,
        "sha": sha,
        "namespace": namespace,
        "caches": caches,
        "durations_seconds": durations,
        "cache_storage": {"configured_limit_bytes": None, "configured_limit_status": "unknown"},
        "resources": {"peak_rss_bytes": None, "cgroup_peak_bytes": None, "oom_events": None},
        "created_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"),
    }
    validate_record(report, source="new cache evidence")
    _private_json(args.output, report)
    _append_summary(args.summary, "CI cache evidence", report)
    return report


def read_record(path: Path) -> dict[str, Any]:
    try:
        if path.stat().st_size > 256 * 1024:
            raise EvidenceError("cache evidence exceeds the bounded record size")
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise EvidenceError(f"cannot read valid cache evidence: {path.name}") from exc
    return validate_record(value, source=path.name)


def compare(cold_path: Path, warm_path: Path) -> dict[str, Any]:
    cold = read_record(cold_path)
    warm = read_record(warm_path)
    identity = ("workflow", "job", "sha", "namespace")
    if any(cold[field] != warm[field] for field in identity):
        raise EvidenceError("cold and warm evidence must have the same workflow, job, SHA, and namespace")
    cold_caches = {item["kind"]: item for item in cold["caches"]}
    warm_caches = {item["kind"]: item for item in warm["caches"]}
    if set(cold_caches) != set(warm_caches):
        raise EvidenceError("cold and warm evidence must cover the same cache kinds")
    caches = []
    for kind in sorted(cold_caches):
        before, after = cold_caches[kind], warm_caches[kind]
        if before["key"] != after["key"]:
            raise EvidenceError("cold and warm cache keys must have the same compatibility identity")
        caches.append(
            {
                "kind": kind,
                "cold_key": before["key"],
                "warm_key": after["key"],
                "cold_restore": before["restore"],
                "warm_restore": after["restore"],
                "cold_save": before["save"],
                "warm_save": after["save"],
                "cold_bytes": before["bytes"],
                "warm_bytes": after["bytes"],
            }
        )
    cold_total = cold["durations_seconds"].get("total")
    warm_total = warm["durations_seconds"].get("total")
    result = {
        "schema_version": SCHEMA_VERSION,
        "workflow": cold["workflow"],
        "job": cold["job"],
        "sha": cold["sha"],
        "namespace": cold["namespace"],
        "caches": caches,
        "durations_seconds": {
            "cold": cold["durations_seconds"],
            "warm": warm["durations_seconds"],
            "delta_total": warm_total - cold_total if cold_total is not None and warm_total is not None else None,
        },
    }
    return result


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    write = commands.add_parser("record", help="write one job's bounded cache evidence")
    write.add_argument("--workflow", required=True)
    write.add_argument("--job", required=True)
    write.add_argument("--sha", required=True)
    write.add_argument("--namespace", required=True)
    write.add_argument("--cache-entry", action="append", required=True, help="kind|primary|matched|hit|restore-outcome|save-outcome|path-bytes")
    write.add_argument("--duration", action="append", default=[], help="install=<seconds>, build=<seconds>, or test=<seconds>; repeat to sum")
    write.add_argument("--output", type=Path, required=True)
    write.add_argument("--summary", type=Path, required=True)
    pair = commands.add_parser("compare", help="compare cold and warm records for one exact candidate")
    pair.add_argument("--cold", type=Path, required=True)
    pair.add_argument("--warm", type=Path, required=True)
    pair.add_argument("--output", type=Path, required=True)
    pair.add_argument("--summary", type=Path, required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        if args.command == "record":
            report = record(args)
        else:
            report = compare(args.cold, args.warm)
            _private_json(args.output, report)
            _append_summary(args.summary, "Cold/warm cache comparison", report)
        print(json.dumps(report, ensure_ascii=False, sort_keys=True))
        return 0
    except (EvidenceError, OSError) as exc:
        print(f"cache evidence error: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
