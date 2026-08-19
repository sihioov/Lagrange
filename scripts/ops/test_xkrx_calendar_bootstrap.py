"""Focused regression checks for the reproducible XKRX bootstrap artifact."""

from __future__ import annotations

import datetime as dt
import json
import importlib.util
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "ops" / "xkrx-calendar-bootstrap.py"
ARTIFACT = ROOT / "data" / "calendars" / "xkrx" / "calendar.json"
MANIFEST = ROOT / "data" / "calendars" / "xkrx" / "manifest.json"


def load_bootstrap_module():
    spec = importlib.util.spec_from_file_location("xkrx_calendar_bootstrap", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load bootstrap module")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class _FakeSchedule:
    def __init__(self, rows):
        self._rows = rows

    def iterrows(self):
        return iter(self._rows)


class _FakeCalendar:
    def __init__(self, rows):
        self.schedule = _FakeSchedule(rows)
        self._labels = [label for label, _row in rows]

    def sessions_in_range(self, _start, _end):
        return self._labels


class _BoundedCalendarProvider:
    def __init__(self):
        self.calls = []

    def get_calendar(self, name, **kwargs):
        self.calls.append((name, kwargs))
        # A call without explicit bounds models the package's dynamic default
        # schedule and intentionally has no 2050 session.
        if not kwargs:
            return _FakeCalendar([])
        opening = dt.datetime(2050, 12, 30, tzinfo=dt.timezone.utc)
        closing = dt.datetime(2050, 12, 30, 6, 30, tzinfo=dt.timezone.utc)
        return _FakeCalendar([(dt.datetime(2050, 12, 30), {"open": opening, "close": closing})])


class _FixedCalendarProvider:
    def __init__(self, rows):
        self.calls = []
        self.rows = rows

    def get_calendar(self, name, **kwargs):
        self.calls.append((name, kwargs))
        return _FakeCalendar(self.rows)


class XkrxCalendarBootstrapTests(unittest.TestCase):
    def test_checked_in_artifact_passes_package_free_check(self) -> None:
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--check", "--end", "2026-08-19"],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("XKRX_CALENDAR_CHECK: PASS", result.stdout)

    def test_third_party_provenance_and_known_closures_are_locked(self) -> None:
        artifact = json.loads(ARTIFACT.read_text(encoding="utf-8"))
        manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
        self.assertEqual(artifact["source"], "exchange_calendars")
        self.assertEqual(artifact["source_version"], "4.13.2")
        self.assertEqual(artifact["source_revision"], "dbe38b1")
        self.assertEqual(artifact["source_license"], "Apache-2.0")
        self.assertEqual(artifact["source_authority"], "third-party-derived")
        self.assertEqual(
            artifact["source_hash"],
            "sha256:fc5a2ad0d61b5c3a6539a3061cd4cbb55c59f4a903455cec7926e4b798919996",
        )
        self.assertEqual(artifact["contract"], "historical-session-dates-only")
        self.assertEqual(artifact["representation"], "dates-only")
        self.assertEqual(artifact["effective_from"], "2020-01-31")
        closed = {entry["date"] for entry in artifact["non_sessions"]}
        for date in ("2020-05-01", "2020-08-17", "2020-12-31"):
            self.assertIn(date, closed, date)
        self.assertEqual(set(artifact["sessions"][0]), {"date", "weekday"})
        exceptional = next(row for row in artifact["source_schedule"] if row["date"] == "2020-12-03")
        self.assertEqual(exceptional["open_local"], "2020-12-03T10:00:00+09:00")
        self.assertEqual(exceptional["close_local"], "2020-12-03T16:30:00+09:00")
        self.assertTrue(all(entry["reason"].startswith("derived:") for entry in artifact["non_sessions"]))
        self.assertEqual(manifest["source"]["revision"], "dbe38b1")
        self.assertEqual(manifest["source"]["license"], "Apache-2.0")
        self.assertEqual(
            manifest["source"]["wheel_sha256"],
            "sha256:fc5a2ad0d61b5c3a6539a3061cd4cbb55c59f4a903455cec7926e4b798919996",
        )
        self.assertEqual(
            manifest["source"]["sdist_sha256"],
            "sha256:a9459425dd64142cd54fbc639847403c7e0c33d60fbc326c94fc1d6bd127f002",
        )

    def test_materialize_requests_explicit_bounds_instead_of_dynamic_default(self) -> None:
        module = load_bootstrap_module()
        provider = _BoundedCalendarProvider()
        artifact = module.materialize(dt.date(2050, 12, 30), dt.date(2050, 12, 31), provider)
        self.assertEqual(provider.calls, [("XKRX", {"start": "2050-12-30", "end": "2050-12-31"})])
        self.assertEqual([row["date"] for row in artifact["sessions"]], ["2050-12-30"])
        self.assertEqual(artifact["non_sessions"][-1]["date"], "2050-12-31")

    def test_materialize_accepts_supported_pre_effective_range_for_unit_contract(self) -> None:
        module = load_bootstrap_module()
        opening = dt.datetime(1998, 12, 4, tzinfo=dt.timezone.utc)
        closing = dt.datetime(1998, 12, 4, 6, 30, tzinfo=dt.timezone.utc)
        later_opening = dt.datetime(1998, 12, 7, tzinfo=dt.timezone.utc)
        later_closing = dt.datetime(1998, 12, 7, 6, 30, tzinfo=dt.timezone.utc)
        provider = _FixedCalendarProvider(
            [
                (dt.datetime(1998, 12, 4), {"open": opening, "close": closing}),
                (dt.datetime(1998, 12, 7), {"open": later_opening, "close": later_closing}),
            ]
        )
        start = dt.date(1998, 12, 4)
        end = dt.date(1998, 12, 12)
        module.validate_supported_range(start, end)
        artifact = module.materialize(start, end, provider)
        self.assertEqual(provider.calls, [("XKRX", {"start": "1998-12-04", "end": "1998-12-12"})])
        self.assertEqual([row["date"] for row in artifact["sessions"]], ["1998-12-04", "1998-12-07"])

    def test_actual_locked_exchange_calendars_handles_weekend_and_holiday_end(self) -> None:
        uv = shutil.which("uv")
        if uv is None:
            self.skipTest("uv is required for the locked upstream regression")
        module_path = str(SCRIPT)
        code = f'''\
import datetime as dt
import importlib.util
import exchange_calendars as xc
spec = importlib.util.spec_from_file_location("bootstrap", {module_path!r})
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
for start, end, expected_sessions, expected_non_sessions in (
    ("2020-01-31", "2020-02-02", ["2020-01-31"], ["2020-02-01", "2020-02-02"]),
    ("2020-12-29", "2020-12-31", ["2020-12-29", "2020-12-30"], ["2020-12-31"]),
    ("1998-12-04", "1998-12-12", [
        "1998-12-04", "1998-12-05", "1998-12-07", "1998-12-08",
        "1998-12-09", "1998-12-10", "1998-12-11",
    ], ["1998-12-06", "1998-12-12"]),
):
    artifact = module.materialize(dt.date.fromisoformat(start), dt.date.fromisoformat(end), xc)
    assert [row["date"] for row in artifact["sessions"]] == expected_sessions
    assert [row["date"] for row in artifact["non_sessions"]] == expected_non_sessions
'''
        result = subprocess.run(
            [uv, "run", "--project", "nt", "--locked", "python", "-c", code],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_emit_sessions_keeps_stdout_dates_only_and_reports_selection_metadata_on_stderr(self) -> None:
        result = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--emit-sessions",
                "--start",
                "2020-01-31",
                "--end",
                "2020-02-02",
            ],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), ["2020-01-31"])
        self.assertNotIn("session_count", result.stdout)
        metadata = json.loads(result.stderr)
        self.assertEqual(metadata["requested_range"], {"start": "2020-01-31", "end": "2020-02-02"})
        self.assertEqual(metadata["session_count"], 1)
        self.assertEqual(metadata["skipped_non_session_count"], 2)
        self.assertEqual(metadata["artifact_sha256"], json.loads(MANIFEST.read_text(encoding="utf-8"))["artifact_sha256"])

    def test_emit_sessions_refuses_outside_materialized_range(self) -> None:
        result = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--emit-sessions",
                "--start",
                "2026-08-19",
                "--end",
                "2026-08-20",
            ],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("outside materialized artifact range", result.stderr)

    def test_emit_sessions_refuses_tampered_artifact_or_manifest(self) -> None:
        with tempfile.TemporaryDirectory(prefix="xkrx-tamper-") as directory:
            output = Path(directory) / "xkrx"
            output.mkdir()
            shutil.copy2(ARTIFACT, output / "calendar.json")
            shutil.copy2(MANIFEST, output / "manifest.json")
            with (output / "calendar.json").open("ab") as stream:
                stream.write(b" ")
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--emit-sessions",
                    "--start",
                    "2020-01-31",
                    "--end",
                    "2020-02-02",
                    "--output-dir",
                    str(output),
                ],
                cwd=ROOT,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("calendar manifest does not match", result.stderr)

    def test_plan_is_side_effect_free_and_start_cannot_predate_effective_date(self) -> None:
        with tempfile.TemporaryDirectory(prefix="xkrx-plan-") as directory:
            output = Path(directory) / "artifact"
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--plan",
                    "--end",
                    "2026-08-19",
                    "--output-dir",
                    str(output),
                ],
                cwd=ROOT,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(output.exists())

        rejected = subprocess.run(
            [sys.executable, str(SCRIPT), "--plan", "--start", "2020-01-30", "--end", "2020-02-03"],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn("effective date", rejected.stderr)


if __name__ == "__main__":
    unittest.main()
