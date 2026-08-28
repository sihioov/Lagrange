#!/usr/bin/env python3
"""Materialize the reviewed exchange_calendars XKRX calendar.

The generator is intentionally a small, deterministic boundary around
``exchange_calendars``.  It never talks to KIS and it never infers a session
from a weekday after generation: both sessions and non-sessions are written
to the artifact and checked as a complete civil-date partition.

``--plan`` is the default and is completely side-effect free.  ``--apply``
always re-executes through the repository's locked ``nt`` environment, so an
operator never has to download a wheel by hand and a globally installed
package cannot accidentally be used.  ``--check`` only reads and validates
the materialized artifact and manifest, so it is usable in a release image
that does not contain Python dependencies.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import hmac
import importlib.metadata
import json
import os
from pathlib import Path
import secrets
import shutil
import stat
import sys
import tempfile
from typing import Any
from zoneinfo import ZoneInfo


PACKAGE_NAME = "exchange_calendars"
PACKAGE_VERSION = "4.13.2"
CALENDAR_NAME = "XKRX"
UPSTREAM_REVISION = "dbe38b1"
UPSTREAM_LICENSE = "Apache-2.0"
TIMEZONE = "Asia/Seoul"
DEFAULT_START = "2016-08-29"
SUPPORTED_START = dt.date(1956, 1, 1)
SUPPORTED_END = dt.date(2050, 12, 31)
ARTIFACT_SCHEMA_VERSION = 3
MANIFEST_SCHEMA_VERSION = 2
ARTIFACT_NAME = "calendar.json"
MANIFEST_NAME = "manifest.json"
OVERRIDE_LEDGER_NAME = "overrides.json"
OVERRIDE_LEDGER_SCHEMA_VERSION = 1
OVERRIDE_LEDGER_CONTRACT = "xkrx-calendar-session-overrides"
ARTIFACT_CONTRACT = "historical-session-dates-only"
CALENDAR_ID = "xkrx-historical-session-dates"
WHEEL_SHA256 = "fc5a2ad0d61b5c3a6539a3061cd4cbb55c59f4a903455cec7926e4b798919996"
SDIST_SHA256 = "a9459425dd64142cd54fbc639847403c7e0c33d60fbc326c94fc1d6bd127f002"
WHEEL_URL = (
    "https://files.pythonhosted.org/packages/c8/4c/0469b40057bc9f8d9594dcc6024202626b981ae4b52dfcd304552e8e1c3a/"
    "exchange_calendars-4.13.2-py3-none-any.whl"
)
SDIST_URL = (
    "https://files.pythonhosted.org/packages/47/73/460b0ece4e7444e3098b7738974c822787e134578d069d3a88b4be17d50a/"
    "exchange_calendars-4.13.2.tar.gz"
)
ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUTPUT_DIR = ROOT / "data" / "calendars" / "xkrx"
UTC = dt.timezone.utc
SEOUL = ZoneInfo(TIMEZONE)

# These are operator-reviewed corrections to the pinned third-party schedule.
# exchange_calendars 4.13.2 remains the raw schedule authority for the
# artifact's audit-only source_schedule; it must not be described as knowing
# these later official closures.  The URLs are retained as auditable evidence
# and the canonical ledger bytes are hashed into both artifacts.
OVERRIDE_LEDGER_ENTRIES: tuple[dict[str, Any], ...] = (
    {
        "date": "2026-06-03",
        "action": "remove_session",
        "reason_code": "national_election_day",
        "reason": "South Korea national election day; KRX holiday rule applies.",
        "sources": (
            {
                "authority": "KRX",
                "url": "https://global.krx.co.kr/contents/GLB/06/0602/0602010201/GLB0602010201T1.jsp",
                "claim": "KRX holiday rule includes national election days.",
            },
            {
                "authority": "National Election Commission",
                "url": "https://www.nec.go.kr/site/nec/ex/bbs/View.do?bcIdx=289351&cbIdx=1104",
                "claim": "The 2026 national election date is 2026-06-03.",
            },
        ),
    },
    {
        "date": "2026-07-17",
        "action": "remove_session",
        "reason_code": "constitution_day_public_holiday",
        "reason": "Constitution Day public holiday restored from 2026; KRX holiday rule applies.",
        "sources": (
            {
                "authority": "KRX",
                "url": "https://global.krx.co.kr/contents/GLB/06/0602/0602010201/GLB0602010201T1.jsp",
                "claim": "KRX holiday rule includes public holidays.",
            },
            {
                "authority": "Korea policy / Ministry of Personnel Management",
                "url": "https://m.korea.kr/news/policyNewsView.do?newsId=148959009",
                "claim": "Constitution Day is a public holiday from 2026.",
            },
        ),
    },
)


class BootstrapError(RuntimeError):
    """A safe, user-actionable bootstrap failure."""


def parse_date(value: str, label: str) -> dt.date:
    try:
        return dt.date.fromisoformat(value)
    except ValueError as exc:
        raise BootstrapError(f"{label} must be YYYY-MM-DD: {value}") from exc


def source_metadata() -> dict[str, Any]:
    artifact = {
        "authority": "third-party-derived",
        "calendar": CALENDAR_NAME,
        "license": UPSTREAM_LICENSE,
        "package": PACKAGE_NAME,
        "revision": UPSTREAM_REVISION,
        "upstream_commit": UPSTREAM_REVISION,
        "version": PACKAGE_VERSION,
        "wheel_sha256": f"sha256:{WHEEL_SHA256}",
        "sdist_sha256": f"sha256:{SDIST_SHA256}",
        "wheel_url": WHEEL_URL,
        "sdist_url": SDIST_URL,
    }
    return artifact


def reexec_with_locked_environment(argv: list[str]) -> None:
    """Run apply in the lockfile-backed environment, downloading automatically.

    A per-process token prevents an ambient environment variable from being
    used as a package-availability bypass.  The child also verifies that its
    interpreter belongs to the checked-in ``nt/.venv`` created by ``uv``.
    ``uv --locked`` verifies the exact artifact hashes from ``nt/uv.lock``.
    """
    uv = shutil.which("uv")
    if uv is None:
        raise BootstrapError(
            f"{PACKAGE_NAME}=={PACKAGE_VERSION} is required for --apply; install uv or "
            "run `uv sync --project nt --locked`"
        )
    project = ROOT / "nt"
    if not (project / "pyproject.toml").is_file() or not (project / "uv.lock").is_file():
        raise BootstrapError("nt/pyproject.toml and nt/uv.lock are required for automatic dependency setup")
    token = secrets.token_hex(32)
    environment = os.environ.copy()
    environment["XKRX_CALENDAR_BOOTSTRAP_REEXEC"] = token
    command = [
        uv,
        "run",
        "--project",
        str(project),
        "--locked",
        "python",
        str(Path(__file__).resolve()),
        "--_locked-child-token",
        token,
        *argv,
    ]
    try:
        os.execvpe(uv, command, environment)
    except OSError as exc:
        raise BootstrapError(f"could not execute locked uv environment: {exc}") from exc


def _locked_child_token_is_valid(token: str | None) -> bool:
    expected = os.environ.get("XKRX_CALENDAR_BOOTSTRAP_REEXEC")
    if not expected or not token or not hmac.compare_digest(expected, token):
        return False
    locked_environment = (ROOT / "nt" / ".venv").resolve()
    if Path(sys.prefix).resolve() != locked_environment:
        return False
    return True


def require_exchange_calendars(argv: list[str], locked_child_token: str | None):
    if not _locked_child_token_is_valid(locked_child_token):
        reexec_with_locked_environment(argv)
    try:
        import exchange_calendars as xc  # type: ignore[import-not-found]
    except ImportError as exc:
        raise BootstrapError(
            f"{PACKAGE_NAME}=={PACKAGE_VERSION} could not be imported after locked setup"
        ) from exc
    try:
        installed_version = importlib.metadata.version(PACKAGE_NAME)
    except importlib.metadata.PackageNotFoundError as exc:
        raise BootstrapError(
            f"{PACKAGE_NAME}=={PACKAGE_VERSION} is not installed in the locked environment"
        ) from exc
    if installed_version != PACKAGE_VERSION or getattr(xc, "__version__", None) != PACKAGE_VERSION:
        raise BootstrapError(
            f"loaded {PACKAGE_NAME} version {installed_version!r}, expected {PACKAGE_VERSION}"
        )
    return xc


def iso_utc(value: Any) -> str:
    """Normalize a pandas timestamp to a canonical UTC RFC3339 string."""

    # pandas.Timestamp exposes ``to_pydatetime`` and ``tz_convert``.  Using
    # these methods keeps the artifact independent of pandas' repr choices.
    if hasattr(value, "tz_convert"):
        value = value.tz_convert("UTC").to_pydatetime()
    elif hasattr(value, "to_pydatetime"):
        value = value.to_pydatetime()
    if not isinstance(value, dt.datetime) or value.tzinfo is None:
        raise BootstrapError(f"calendar returned a timezone-naive timestamp: {value!r}")
    if value.microsecond:
        raise BootstrapError(f"calendar returned a sub-second timestamp: {value!r}")
    value = value.astimezone(UTC).replace(microsecond=0)
    return value.isoformat().replace("+00:00", "Z")


def iso_local(value: Any) -> str:
    if hasattr(value, "tz_convert"):
        value = value.tz_convert(TIMEZONE).to_pydatetime()
    elif hasattr(value, "to_pydatetime"):
        value = value.to_pydatetime()
    if not isinstance(value, dt.datetime) or value.tzinfo is None:
        raise BootstrapError(f"calendar returned a timezone-naive timestamp: {value!r}")
    if value.microsecond:
        raise BootstrapError(f"calendar returned a sub-second timestamp: {value!r}")
    return value.astimezone(SEOUL).replace(microsecond=0).isoformat()


def iso_optional_utc(value: Any) -> str | None:
    if value is None:
        return None
    try:
        if value != value:  # pandas NaT
            return None
    except (TypeError, ValueError):
        pass
    return iso_utc(value)


def iso_optional_local(value: Any) -> str | None:
    if value is None:
        return None
    try:
        if value != value:  # pandas NaT
            return None
    except (TypeError, ValueError):
        pass
    return iso_local(value)


def weekday_name(value: dt.date) -> str:
    return value.strftime("%A")


def validate_supported_range(start: dt.date, end: dt.date) -> None:
    """Validate the upstream XKRX support bounds before querying it."""

    if end < start:
        raise BootstrapError("requested calendar range is reversed")
    if start < SUPPORTED_START or end > SUPPORTED_END:
        raise BootstrapError(
            f"requested range must stay within exchange_calendars {CALENDAR_NAME} bounds "
            f"{SUPPORTED_START}..{SUPPORTED_END}"
        )


def validate_requested_range(start: dt.date, end: dt.date) -> None:
    """Validate the supported bounds and this repository's fixed universe."""

    validate_supported_range(start, end)
    effective_from = dt.date.fromisoformat(DEFAULT_START)
    if start < effective_from:
        raise BootstrapError(f"requested start must be on/after fixed-universe effective date {DEFAULT_START}")


def schedule_date(label: Any) -> dt.date:
    value = label.date() if hasattr(label, "date") else dt.date.fromisoformat(str(label)[:10])
    if not isinstance(value, dt.date):
        raise BootstrapError(f"calendar returned an invalid session label: {label!r}")
    return value


def materialize(start: dt.date, end: dt.date, xc: Any) -> dict[str, Any]:
    # Do this before get_calendar: exchange_calendars has a dynamic default
    # schedule range, and using it without explicit bounds can turn dates past
    # that range into false non-sessions.
    validate_supported_range(start, end)
    query_start = start
    query_end = end
    try:
        try:
            calendar = xc.get_calendar(CALENDAR_NAME, start=str(start), end=str(end))
        except Exception:
            # exchange_calendars 4.13.2 rejects a one-civil-day interval even
            # though the public contract is inclusive.  Preserve the required
            # exact bounded call above, then widen only that degenerate query
            # by a small bounded neighborhood.  A one-day request can itself
            # be a weekend/closure, so one neighboring civil day is not
            # guaranteed to contain a session.
            if start != end:
                raise
            query_start = max(SUPPORTED_START, start - dt.timedelta(days=7))
            query_end = min(SUPPORTED_END, end + dt.timedelta(days=7))
            calendar = xc.get_calendar(CALENDAR_NAME, start=str(query_start), end=str(query_end))
        schedule = calendar.schedule
    except Exception as exc:  # pandas/exchange_calendars errors are not stable API types
        raise BootstrapError(f"{CALENDAR_NAME} schedule generation failed: {exc}") from exc

    sessions: list[dict[str, Any]] = []
    source_schedule: list[dict[str, Any]] = []
    session_dates: set[str] = set()
    for label, row in schedule.iterrows():
        date_value = schedule_date(label)
        if date_value < query_start or date_value > query_end:
            raise BootstrapError(
                f"{CALENDAR_NAME} returned session {date_value} outside bounded query {query_start}..{query_end}"
            )
        if date_value < start or date_value > end:
            continue
        date_text = date_value.isoformat()
        if date_text in session_dates:
            raise BootstrapError(f"exchange_calendars returned duplicate session {date_text}")
        session_dates.add(date_text)
        try:
            opened = row["open"]
            closed = row["close"]
        except (KeyError, TypeError) as exc:
            raise BootstrapError(f"{CALENDAR_NAME} schedule has no open/close for {date_text}") from exc
        sessions.append({"date": date_text, "weekday": weekday_name(date_value)})
        # Keep exact upstream instants as audit evidence, but deliberately put
        # them outside the dates-only session contract.  Rust publication and
        # curation must never consume or flatten these values into 09:00/15:30.
        source_schedule.append(
            {
                "date": date_text,
                "open_local": iso_local(opened),
                "close_local": iso_local(closed),
                "open_utc": iso_utc(opened),
                "close_utc": iso_utc(closed),
                "break_start_local": iso_optional_local(row.get("break_start")),
                "break_end_local": iso_optional_local(row.get("break_end")),
                "break_start_utc": iso_optional_utc(row.get("break_start")),
                "break_end_utc": iso_optional_utc(row.get("break_end")),
            }
        )

    schedule_dates = [entry["date"] for entry in sessions]
    if schedule_dates != sorted(schedule_dates) or len(schedule_dates) != len(set(schedule_dates)):
        raise BootstrapError(f"{CALENDAR_NAME} returned an unsorted or duplicate schedule")

    non_sessions: list[dict[str, str]] = []
    cursor = start
    while cursor <= end:
        date_text = cursor.isoformat()
        if date_text not in session_dates:
            reason = "derived:weekend" if cursor.weekday() >= 5 else "derived:exchange_calendars_closure"
            non_sessions.append(
                {"date": date_text, "weekday": weekday_name(cursor), "reason": reason}
            )
        cursor += dt.timedelta(days=1)

    artifact = {
        "artifact_schema_version": ARTIFACT_SCHEMA_VERSION,
        "contract": ARTIFACT_CONTRACT,
        "representation": "dates-only",
        "calendar_id": CALENDAR_ID,
        "exchange": "KRX",
        "source": PACKAGE_NAME,
        "source_version": PACKAGE_VERSION,
        "source_hash": f"sha256:{WHEEL_SHA256}",
        "source_revision": UPSTREAM_REVISION,
        "source_upstream_commit": UPSTREAM_REVISION,
        "source_license": UPSTREAM_LICENSE,
        "source_authority": "third-party-derived",
        "timezone": TIMEZONE,
        "utc_offset": "+09:00",
        "effective_from": DEFAULT_START,
        "range": {"start": start.isoformat(), "end": end.isoformat()},
        "sessions": sessions,
        "non_sessions": non_sessions,
        # Keep the established market-data spelling available to consumers;
        # unlike the old fixture, this list is a complete date partition and
        # includes weekends with an explicit reason.
        "holidays": non_sessions,
        "source_schedule": source_schedule,
        "source_schedule_purpose": "audit-only; not a publication or curation calendar",
    }
    return apply_calendar_overrides(artifact)


def canonical_bytes(value: dict[str, Any]) -> bytes:
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")


def override_ledger_value() -> dict[str, Any]:
    # Round-trip through JSON so the in-code tuple constants have exactly the
    # same list/object shape as the tracked JSON ledger.
    return json.loads(
        json.dumps(
            {
                "schema_version": OVERRIDE_LEDGER_SCHEMA_VERSION,
                "contract": OVERRIDE_LEDGER_CONTRACT,
                "calendar_id": CALENDAR_ID,
                "authority": "operator-reviewed official sources",
                "entries": list(OVERRIDE_LEDGER_ENTRIES),
            },
            ensure_ascii=False,
            sort_keys=True,
        )
    )


def override_ledger_bytes() -> bytes:
    return canonical_bytes(override_ledger_value())


def override_ledger_metadata() -> dict[str, Any]:
    payload = override_ledger_bytes()
    return {
        "name": OVERRIDE_LEDGER_NAME,
        "schema_version": OVERRIDE_LEDGER_SCHEMA_VERSION,
        "contract": OVERRIDE_LEDGER_CONTRACT,
        "sha256": f"sha256:{sha256_bytes(payload)}",
        "size_bytes": len(payload),
    }


def apply_calendar_overrides(artifact: dict[str, Any]) -> dict[str, Any]:
    ledger = override_ledger_value()
    artifact_range = artifact["range"]
    start = parse_date(artifact_range["start"], "artifact range start")
    end = parse_date(artifact_range["end"], "artifact range end")
    entries = [
        entry
        for entry in ledger["entries"]
        if start <= parse_date(entry["date"], "override date") <= end
    ]
    session_by_date = {entry["date"]: entry for entry in artifact["sessions"]}
    for override in entries:
        date_text = override["date"]
        if date_text not in session_by_date:
            raise BootstrapError(
                f"calendar override {date_text} is not present in the raw {CALENDAR_NAME} schedule"
            )
        artifact["sessions"] = [entry for entry in artifact["sessions"] if entry["date"] != date_text]
        artifact["non_sessions"].append(
            {
                "date": date_text,
                "weekday": weekday_name(parse_date(date_text, "override date")),
                "reason": f"override:{override['reason_code']}",
            }
        )
    artifact["sessions"].sort(key=lambda entry: entry["date"])
    artifact["non_sessions"].sort(key=lambda entry: entry["date"])
    artifact["holidays"] = list(artifact["non_sessions"])
    artifact["override_ledger"] = override_ledger_metadata()
    artifact["override_entries"] = entries
    return artifact


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def validate_artifact(artifact: dict[str, Any], expected_start: dt.date, expected_end: dt.date) -> None:
    required = {
        "artifact_schema_version",
        "contract",
        "representation",
        "calendar_id",
        "exchange",
        "source",
        "source_version",
        "source_hash",
        "source_revision",
        "source_license",
        "source_authority",
        "timezone",
        "utc_offset",
        "range",
        "effective_from",
        "sessions",
        "non_sessions",
        "holidays",
        "source_schedule",
        "source_schedule_purpose",
        "override_ledger",
        "override_entries",
    }
    missing = sorted(required - artifact.keys())
    if missing:
        raise BootstrapError(f"calendar artifact is missing fields: {', '.join(missing)}")
    if artifact["artifact_schema_version"] != ARTIFACT_SCHEMA_VERSION:
        raise BootstrapError("unsupported calendar artifact schema")
    if artifact["contract"] != ARTIFACT_CONTRACT or artifact["representation"] != "dates-only":
        raise BootstrapError("calendar artifact is not the historical dates-only contract")
    if artifact["calendar_id"] != CALENDAR_ID or artifact["exchange"] != "KRX":
        raise BootstrapError("calendar artifact has an unexpected exchange identity")
    if artifact["source"] != PACKAGE_NAME or artifact["source_version"] != PACKAGE_VERSION:
        raise BootstrapError("calendar artifact source/version does not match the pinned dependency")
    if artifact["source_hash"] != f"sha256:{WHEEL_SHA256}":
        raise BootstrapError("calendar artifact source hash does not match the pinned wheel")
    if artifact["source_revision"] != UPSTREAM_REVISION:
        raise BootstrapError("calendar artifact upstream revision does not match the reviewed source")
    if artifact.get("source_upstream_commit") != UPSTREAM_REVISION:
        raise BootstrapError("calendar artifact upstream commit does not match the reviewed source")
    if artifact["source_license"] != UPSTREAM_LICENSE:
        raise BootstrapError("calendar artifact upstream license does not match the reviewed source")
    if artifact["source_authority"] != "third-party-derived":
        raise BootstrapError("calendar artifact must not claim official/KIS authority")
    if artifact["effective_from"] != DEFAULT_START:
        raise BootstrapError("calendar artifact effective_from must remain 2016-08-29")
    if artifact["timezone"] != TIMEZONE:
        raise BootstrapError("calendar artifact timezone must be Asia/Seoul")
    if artifact["utc_offset"] != "+09:00":
        raise BootstrapError("calendar artifact UTC offset must remain +09:00")
    if artifact["source_authority"] != "third-party-derived":
        raise BootstrapError("calendar artifact must not claim official/KIS authority")
    if artifact["source_schedule_purpose"] != "audit-only; not a publication or curation calendar":
        raise BootstrapError("calendar source schedule must remain audit-only")
    expected_ledger = override_ledger_value()
    expected_override_entries = [
        entry
        for entry in expected_ledger["entries"]
        if expected_start <= parse_date(entry["date"], "override date") <= expected_end
    ]
    if artifact["override_entries"] != expected_override_entries:
        raise BootstrapError("calendar override entries do not match the reviewed ledger")
    if artifact["override_ledger"] != override_ledger_metadata():
        raise BootstrapError("calendar override ledger metadata does not match the reviewed ledger")
    try:
        artifact_range = artifact["range"]
        if not isinstance(artifact_range, dict):
            raise TypeError
        artifact_start = parse_date(artifact_range["start"], "artifact range start")
        artifact_end = parse_date(artifact_range["end"], "artifact range end")
    except (KeyError, TypeError):
        raise BootstrapError("calendar artifact range is malformed")
    if (artifact_start, artifact_end) != (expected_start, expected_end):
        raise BootstrapError(
            f"calendar artifact range is {artifact_start}..{artifact_end}, expected {expected_start}..{expected_end}"
        )
    validate_requested_range(artifact_start, artifact_end)

    sessions = artifact["sessions"]
    non_sessions = artifact["non_sessions"]
    holidays = artifact["holidays"]
    source_schedule = artifact["source_schedule"]
    if (
        not isinstance(sessions, list)
        or not isinstance(non_sessions, list)
        or not isinstance(holidays, list)
        or not isinstance(source_schedule, list)
    ):
        raise BootstrapError("calendar sessions/non-sessions/holidays must be arrays")
    if any(not isinstance(entry, dict) for entry in sessions + non_sessions + holidays + source_schedule):
        raise BootstrapError("calendar entries must be objects")
    session_dates = [entry.get("date") for entry in sessions]
    non_session_dates = [entry.get("date") for entry in non_sessions]
    if session_dates != sorted(session_dates) or len(session_dates) != len(set(session_dates)):
        raise BootstrapError("calendar sessions are not sorted and unique")
    if non_session_dates != sorted(non_session_dates) or len(non_session_dates) != len(set(non_session_dates)):
        raise BootstrapError("calendar non-sessions are not sorted and unique")
    if holidays != non_sessions:
        raise BootstrapError("calendar holidays must exactly mirror non-sessions")
    expected_dates = {
        (expected_start + dt.timedelta(days=offset)).isoformat()
        for offset in range((expected_end - expected_start).days + 1)
    }
    if set(session_dates) | set(non_session_dates) != expected_dates:
        raise BootstrapError("calendar sessions and non-sessions do not cover the requested range")
    if set(session_dates) & set(non_session_dates):
        raise BootstrapError("calendar date is both a session and non-session")
    for entry in sessions:
        date_value = parse_date(entry.get("date", ""), "session date")
        if entry.get("weekday") != weekday_name(date_value):
            raise BootstrapError(f"session weekday is wrong for {date_value}")
        if date_value.weekday() >= 5:
            raise BootstrapError(f"weekend appears as a session: {date_value}")
    for entry in non_sessions:
        date_value = parse_date(entry.get("date", ""), "non-session date")
        if entry.get("weekday") != weekday_name(date_value):
            raise BootstrapError(f"non-session weekday is wrong for {date_value}")
        if entry.get("reason") not in {
            "derived:weekend",
            "derived:exchange_calendars_closure",
            "override:national_election_day",
            "override:constitution_day_public_holiday",
        }:
            raise BootstrapError(f"non-session {date_value} has an unknown reason")

    source_dates = [entry.get("date") for entry in source_schedule]
    override_dates = {entry["date"] for entry in expected_override_entries}
    if source_dates != sorted(set(session_dates) | override_dates):
        raise BootstrapError(
            "audit source schedule must preserve raw upstream sessions, including overridden dates"
        )
    if not override_dates.issubset(set(source_dates)):
        raise BootstrapError("every calendar override must correspond to a raw upstream session")
    for entry in source_schedule:
        date_value = parse_date(entry.get("date", ""), "source schedule date")
        parsed: dict[str, dt.datetime | None] = {}
        for field in ("open_utc", "close_utc", "open_local", "close_local"):
            try:
                raw = entry[field]
                parsed[field] = dt.datetime.fromisoformat(raw.replace("Z", "+00:00"))
            except (KeyError, AttributeError, TypeError, ValueError) as exc:
                raise BootstrapError(f"invalid source schedule {field} for {date_value}") from exc
        opened = parsed["open_utc"]
        closed = parsed["close_utc"]
        assert opened is not None and closed is not None
        if opened.tzinfo is None or opened.utcoffset() != dt.timedelta(0):
            raise BootstrapError(f"source schedule open_utc is not UTC for {date_value}")
        if closed.tzinfo is None or closed.utcoffset() != dt.timedelta(0):
            raise BootstrapError(f"source schedule close_utc is not UTC for {date_value}")
        if opened.date() != date_value or closed.date() != date_value or opened >= closed:
            raise BootstrapError(f"source schedule UTC interval is invalid for {date_value}")
        for field in ("open_local", "close_local"):
            local = parsed[field]
            assert local is not None
            if local.tzinfo is None or local.utcoffset() != dt.timedelta(hours=9):
                raise BootstrapError(f"source schedule {field} is not +09:00 for {date_value}")
            if local.date() != date_value:
                raise BootstrapError(f"source schedule {field} has the wrong civil date for {date_value}")
        break_start = _parse_optional_source_timestamp(entry, "break_start_utc", date_value, UTC)
        break_end = _parse_optional_source_timestamp(entry, "break_end_utc", date_value, UTC)
        break_start_local = _parse_optional_source_timestamp(entry, "break_start_local", date_value, SEOUL)
        break_end_local = _parse_optional_source_timestamp(entry, "break_end_local", date_value, SEOUL)
        if (break_start is None) != (break_end is None) or (break_start_local is None) != (break_end_local is None):
            raise BootstrapError(f"source schedule break interval is incomplete for {date_value}")
        if break_start is not None and break_end is not None and break_start >= break_end:
            raise BootstrapError(f"source schedule break interval is reversed for {date_value}")


def _parse_optional_source_timestamp(
    entry: dict[str, Any], field: str, date_value: dt.date, zone: dt.tzinfo
) -> dt.datetime | None:
    raw = entry.get(field)
    if raw is None:
        return None
    if not isinstance(raw, str):
        raise BootstrapError(f"source schedule {field} is not a string for {date_value}")
    try:
        parsed = dt.datetime.fromisoformat(raw.replace("Z", "+00:00"))
    except ValueError as exc:
        raise BootstrapError(f"invalid source schedule {field} for {date_value}") from exc
    expected_offset = dt.datetime.combine(date_value, dt.time(), zone).utcoffset()
    if parsed.tzinfo is None or parsed.utcoffset() != expected_offset:
        raise BootstrapError(f"source schedule {field} has the wrong timezone for {date_value}")
    if parsed.date() != date_value:
        raise BootstrapError(f"source schedule {field} has the wrong civil date for {date_value}")
    return parsed


def build_manifest(artifact: dict[str, Any], artifact_bytes: bytes) -> dict[str, Any]:
    return {
        "manifest_schema_version": MANIFEST_SCHEMA_VERSION,
        "artifact": ARTIFACT_NAME,
        "artifact_sha256": f"sha256:{sha256_bytes(artifact_bytes)}",
        "artifact_size_bytes": len(artifact_bytes),
        "contract": artifact["contract"],
        "calendar_id": artifact["calendar_id"],
        "exchange": artifact["exchange"],
        "timezone": artifact["timezone"],
        "effective_from": artifact["effective_from"],
        "range": artifact["range"],
        "session_count": len(artifact["sessions"]),
        "non_session_count": len(artifact["non_sessions"]),
        "override_ledger": artifact["override_ledger"],
        "source": source_metadata(),
        "generator": "scripts/ops/xkrx-calendar-bootstrap.py",
    }


def reject_unsafe_output(path: Path) -> None:
    # Check the lexical path before resolving it.  Resolving first would hide
    # a symlinked parent (for example ``/var/lib/alias/xkrx``) and could make a
    # caller validate a calendar outside the checked-in directory by accident.
    if not path.is_absolute():
        raise BootstrapError(f"output directory must be absolute: {path}")
    if any(part == ".." for part in path.parts):
        raise BootstrapError(f"output directory must not contain '..': {path}")
    current = Path(path.anchor)
    for part in path.parts[1:]:
        current /= part
        if current.is_symlink():
            raise BootstrapError(f"output path traverses a symlink: {current}")
    resolved = path.resolve()
    if resolved in {Path("/"), Path.home()}:
        raise BootstrapError(f"refusing to use a broad output directory: {resolved}")
    if path.exists() and not path.is_dir():
        raise BootstrapError(f"output path is not a directory: {path}")


def atomic_write(path: Path, payload: bytes, replace: bool) -> None:
    if path.is_symlink():
        raise BootstrapError(f"refusing to replace symlink: {path}")
    if path.exists() and not replace:
        existing = path.read_bytes()
        if existing == payload:
            return
        raise BootstrapError(f"{path} differs; pass --replace only after reviewing the new artifact")
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, stat.S_IRUSR | stat.S_IWUSR | stat.S_IRGRP | stat.S_IROTH)
        os.replace(temporary, path)
    except Exception:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise


def load_existing(output_dir: Path, start: dt.date, end: dt.date) -> tuple[dict[str, Any], dict[str, Any], bytes]:
    artifact_path = output_dir / ARTIFACT_NAME
    manifest_path = output_dir / MANIFEST_NAME
    override_path = output_dir / OVERRIDE_LEDGER_NAME
    if not artifact_path.is_file() or artifact_path.is_symlink():
        raise BootstrapError(f"missing or unsafe artifact: {artifact_path}")
    if not manifest_path.is_file() or manifest_path.is_symlink():
        raise BootstrapError(f"missing or unsafe manifest: {manifest_path}")
    if not override_path.is_file() or override_path.is_symlink():
        raise BootstrapError(f"missing or unsafe override ledger: {override_path}")
    artifact_bytes = artifact_path.read_bytes()
    override_bytes = override_path.read_bytes()
    if override_bytes != override_ledger_bytes():
        raise BootstrapError("override ledger does not match the reviewed source-backed bytes")
    try:
        artifact = json.loads(artifact_bytes.decode("utf-8"))
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise BootstrapError(f"calendar artifact or manifest is not valid UTF-8 JSON: {exc}") from exc
    if not isinstance(artifact, dict) or not isinstance(manifest, dict):
        raise BootstrapError("calendar artifact and manifest must be JSON objects")
    validate_artifact(artifact, start, end)
    expected_manifest = build_manifest(artifact, artifact_bytes)
    if manifest != expected_manifest:
        raise BootstrapError("calendar manifest does not match the artifact or pinned source metadata")
    return artifact, manifest, artifact_bytes


def run_plan(start: dt.date, end: dt.date, output_dir: Path) -> None:
    print("XKRX_CALENDAR_PLAN")
    print(f"  range={start.isoformat()}..{end.isoformat()}")
    print(f"  output={output_dir}")
    print(f"  contract={ARTIFACT_CONTRACT} representation=dates-only")
    print(f"  source={PACKAGE_NAME}=={PACKAGE_VERSION} calendar={CALENDAR_NAME}")
    print(f"  provenance=third-party-derived revision={UPSTREAM_REVISION} license={UPSTREAM_LICENSE}")
    print(f"  wheel_sha256=sha256:{WHEEL_SHA256}")
    print(f"  sdist_sha256=sha256:{SDIST_SHA256}")
    print("  artifact=calendar.json manifest=manifest.json")
    print("  modes=plan|apply|check; apply always runs via uv --project nt --locked")
    print("PLAN_ONLY: no package download, KIS call, Docker lifecycle, secret, or file write")


def run_apply(
    start: dt.date,
    end: dt.date,
    output_dir: Path,
    argv: list[str],
    replace: bool,
    locked_child_token: str | None,
) -> None:
    reject_unsafe_output(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    xc = require_exchange_calendars(argv, locked_child_token)
    artifact = materialize(start, end, xc)
    validate_artifact(artifact, start, end)
    artifact_bytes = canonical_bytes(artifact)
    manifest = build_manifest(artifact, artifact_bytes)
    manifest_bytes = canonical_bytes(manifest)
    ledger_bytes = override_ledger_bytes()
    # Refuse a changed existing artifact unless the operator explicitly opted
    # into replacement; identical reruns remain idempotent and write nothing.
    atomic_write(output_dir / ARTIFACT_NAME, artifact_bytes, replace)
    atomic_write(output_dir / MANIFEST_NAME, manifest_bytes, replace)
    atomic_write(output_dir / OVERRIDE_LEDGER_NAME, ledger_bytes, replace)
    print(
        f"XKRX_CALENDAR_APPLY: PASS range={start.isoformat()}..{end.isoformat()} "
        f"sessions={len(artifact['sessions'])} non_sessions={len(artifact['non_sessions'])} "
        f"artifact_sha256=sha256:{sha256_bytes(artifact_bytes)}"
    )


def run_check(start: dt.date, end: dt.date, output_dir: Path) -> None:
    reject_unsafe_output(output_dir)
    artifact, manifest, artifact_bytes = load_existing(output_dir, start, end)
    print(
        f"XKRX_CALENDAR_CHECK: PASS range={manifest['range']['start']}..{manifest['range']['end']} "
        f"sessions={len(artifact['sessions'])} non_sessions={len(artifact['non_sessions'])} "
        f"artifact_sha256={manifest['artifact_sha256']}"
    )


def _validated_selection(
    start: dt.date, end: dt.date, output_dir: Path
) -> tuple[dict[str, Any], dict[str, Any]]:
    """Load the checked-in calendar for a scheduler-only date selection.

    This deliberately shares the full artifact/manifest validator with
    ``--check``.  It does not import exchange_calendars and it never exposes
    the audit-only source schedule to callers.
    """

    reject_unsafe_output(output_dir)
    # ``load_existing`` normally validates an exact checked-in range.  A
    # scheduler selection is allowed to be an inclusive subrange, so first
    # recover the artifact's own range from the untrusted JSON and then run the
    # same complete validator against that exact range before filtering it.
    artifact_path = output_dir / ARTIFACT_NAME
    manifest_path = output_dir / MANIFEST_NAME
    if not artifact_path.is_file() or artifact_path.is_symlink():
        raise BootstrapError(f"missing or unsafe artifact: {artifact_path}")
    if not manifest_path.is_file() or manifest_path.is_symlink():
        raise BootstrapError(f"missing or unsafe manifest: {manifest_path}")
    try:
        candidate = json.loads(artifact_path.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise BootstrapError(f"calendar artifact is not valid UTF-8 JSON: {exc}") from exc
    if not isinstance(candidate, dict):
        raise BootstrapError("calendar artifact must be a JSON object")
    try:
        candidate_range = candidate["range"]
        artifact_start = parse_date(candidate_range["start"], "artifact range start")
        artifact_end = parse_date(candidate_range["end"], "artifact range end")
    except (KeyError, TypeError, ValueError) as exc:
        raise BootstrapError("calendar artifact range is malformed") from exc
    artifact, manifest, _ = load_existing(output_dir, artifact_start, artifact_end)
    if start < artifact_start or end > artifact_end:
        raise BootstrapError(
            f"requested selection {start}..{end} is outside materialized artifact range "
            f"{artifact_start}..{artifact_end}"
        )
    return artifact, manifest


def run_emit_sessions(start: dt.date, end: dt.date, output_dir: Path) -> None:
    artifact, manifest = _validated_selection(start, end, output_dir)
    selected_sessions = [
        entry
        for entry in artifact["sessions"]
        if start <= parse_date(entry["date"], "session date") <= end
    ]
    selected_non_sessions = [
        entry
        for entry in artifact["non_sessions"]
        if start <= parse_date(entry["date"], "non-session date") <= end
    ]
    metadata = {
        "schema": "xkrx-historical-session-selection-v1",
        "contract": artifact["contract"],
        "representation": artifact["representation"],
        "calendar_id": manifest["calendar_id"],
        "artifact_sha256": manifest["artifact_sha256"],
        "artifact_size_bytes": manifest["artifact_size_bytes"],
        "source": manifest["source"],
        "artifact_range": manifest["range"],
        "requested_range": {"start": start.isoformat(), "end": end.isoformat()},
        "session_count": len(selected_sessions),
        "skipped_non_session_count": len(selected_non_sessions),
    }
    # Keep stdout strictly dates-only for a scheduler pipe.  The caller can
    # capture stderr in memory for the identity/count metadata.
    print(
        json.dumps(metadata, ensure_ascii=False, sort_keys=True, separators=(",", ":")),
        file=sys.stderr,
    )
    # stdout is a deliberately boring, package-free wire format: one
    # validated civil date per line and no open/close or reason fields.
    for entry in selected_sessions:
        print(entry["date"])


def parser() -> argparse.ArgumentParser:
    command = argparse.ArgumentParser(description=__doc__)
    modes = command.add_mutually_exclusive_group()
    modes.add_argument("--plan", action="store_true", help="print a no-change plan (default)")
    modes.add_argument("--apply", action="store_true", help="materialize the artifact")
    modes.add_argument("--check", action="store_true", help="validate an existing artifact")
    modes.add_argument(
        "--emit-sessions",
        action="store_true",
        help="emit only validated session dates for the historical scheduler",
    )
    command.add_argument("--start", default=DEFAULT_START, help=f"inclusive start date (default: {DEFAULT_START})")
    command.add_argument("--end", required=True, help="inclusive end date; an explicit end is required")
    command.add_argument("--output-dir", type=Path, default=DEFAULT_OUTPUT_DIR)
    command.add_argument("--replace", action="store_true", help="allow replacing a changed artifact during --apply")
    command.add_argument("--_locked-child-token", help=argparse.SUPPRESS)
    return command


def main(argv: list[str]) -> int:
    args = parser().parse_args(argv)
    try:
        start = parse_date(args.start, "--start")
        end = parse_date(args.end, "--end")
        if end < start:
            raise BootstrapError("--end precedes --start")
        validate_requested_range(start, end)
        output_dir = args.output_dir
        if args.apply:
            run_apply(start, end, output_dir, argv, args.replace, args._locked_child_token)
        elif args.check:
            run_check(start, end, output_dir)
        elif args.emit_sessions:
            run_emit_sessions(start, end, output_dir)
        else:
            run_plan(start, end, output_dir)
        return 0
    except BootstrapError as exc:
        print(f"xkrx-calendar-bootstrap: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
