import fcntl
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import uuid


SCRIPT = Path(__file__).with_name("kis-calendar-coverage.py")
ENTITLEMENT = "fixture:licensed"
ENDPOINT = "/uapi/domestic-stock/v1/quotations/chk-holiday"
FILE_NAME = "calendar-page-01.json"


def digest(wire):
    return "sha256:" + hashlib.sha256(wire).hexdigest()


class CalendarInventoryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.raw = self.root / "raw"
        for provider in ("kis", "kis-calendar"):
            scope = self.scope(provider)
            scope.mkdir(parents=True)
            (scope / "commit.lock").touch()
            (scope / "manifest.jsonl").touch()

    def scope(self, provider):
        return self.raw / "manifests" / f"provider={provider}" / "market=kr"

    def batch(self, provider, partition_date, request_date, dates, *, copy=None,
              entitlement=ENTITLEMENT, committed=True, file_name=FILE_NAME):
        batch_id = str(uuid.uuid4())
        retrieval = f"{partition_date}T08:00:00Z"
        body = json.dumps({
            "rt_cd": "0",
            "output": [
                {"bass_dt": day.replace("-", ""), "opnd_yn": opened} for day, opened in dates
            ],
            "msg1": "fixture body must stay private",
        }).encode()
        file = {
            "kind": "calendar",
            "file_name": file_name,
            "content_hash": digest(body),
            "size_bytes": len(body),
            "request": {
                "endpoint": ENDPOINT,
                "mode": "credentialed",
                "query": [
                    ["BASS_DT", request_date.replace("-", "")], ["CTX_AREA_FK", ""],
                    ["CTX_AREA_NK", ""],
                ],
                "headers": [
                    ["authorization", "[REDACTED]"], ["appkey", "[REDACTED]"],
                    ["appsecret", "[REDACTED]"], ["tr_id", "CTCA0903R"], ["tr_cont", ""],
                ],
            },
            "response_continuation": "E",
        }
        if copy is not None:
            file["copied_from"] = copy
        record = {
            "batch_id": batch_id, "provider": provider, "market": "kr", "date": partition_date,
            "retrieved_at": retrieval, "mode": "credentialed", "entitlement_reference": entitlement,
            "files": [file],
        }
        batch_dir = self.raw / f"provider={provider}" / "market=kr" / f"date={partition_date}"
        batch_dir = batch_dir / f"batch={batch_id}"
        batch_dir.mkdir(parents=True)
        (batch_dir / "batch.json").write_text(json.dumps(record, indent=2), encoding="utf-8")
        (batch_dir / file_name).write_bytes(body)
        if committed:
            with (self.scope(provider) / "manifest.jsonl").open("a", encoding="utf-8") as manifest:
                manifest.write(json.dumps(record) + "\n")
        return batch_dir / file_name, {
            "provider": provider, "market": "kr", "batch_id": batch_id, "file_name": file_name,
            "content_hash": digest(body), "retrieved_at": retrieval,
        }

    def run_inventory(self, through="2026-09-05", *, code=0):
        result = subprocess.run([
            sys.executable, "-I", "-S", "-B", str(SCRIPT), "--data-root", str(self.root),
            "--through", through, "--entitlement-sha256", digest(ENTITLEMENT.encode()),
        ], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, code, result.stdout + result.stderr)
        self.assertEqual(result.stderr, "")
        self.assertNotIn(ENTITLEMENT, result.stdout)
        self.assertNotIn("fixture body must stay private", result.stdout)
        self.assertEqual(len(result.stdout.splitlines()), 1)
        return json.loads(result.stdout)

    def test_uses_returned_dates_excludes_uncommitted_and_other_entitlements(self):
        dates = [("2026-08-29", "N"), ("2026-08-30", "N"), ("2026-08-31", "Y")]
        _, original = self.batch("kis-calendar", "2026-08-29", "2026-08-29", dates,
                                 file_name="calendar.json")
        self.batch("kis", "2026-08-31", "2026-08-29", dates, copy=original)
        self.batch("kis", "2026-09-03", "2026-09-03", [
            ("2026-09-03", "Y"), ("2026-09-04", "Y"), ("2026-09-06", "N"),
        ])
        self.batch("kis", "2026-09-04", "2026-09-03", [
            ("2026-09-03", "Y"), ("2026-09-04", "Y"), ("2026-09-06", "N"),
        ])
        self.batch("kis-calendar", "2026-09-01", "2026-09-01", [("2026-09-01", "Y")],
                   entitlement="fixture:other")
        self.batch("kis-calendar", "2026-09-02", "2026-09-02", [("2026-09-02", "Y")],
                   committed=False)
        before = {path: path.read_bytes() for path in self.raw.rglob("*") if path.is_file()}
        result = self.run_inventory()
        self.assertEqual(result, {
            "status": "inventory", "required_start": "2026-08-29", "required_end": "2026-09-05",
            "missing_days": 3,
            "missing_intervals": [
                {"start": "2026-09-01", "end": "2026-09-02"},
                {"start": "2026-09-05", "end": "2026-09-05"},
            ],
            "original_responses": 2,
            "response_windows": [
                {"request_date": "2026-08-29", "first_date": "2026-08-29",
                 "last_date": "2026-08-31", "unique_dates": 3, "responses": 1},
                {"request_date": "2026-09-03", "first_date": "2026-09-03",
                 "last_date": "2026-09-06", "unique_dates": 3, "responses": 1},
            ],
        })
        self.assertEqual(before, {path: path.read_bytes() for path in self.raw.rglob("*")
                                  if path.is_file()})

    def test_fails_closed_on_tampered_committed_bytes(self):
        path, _ = self.batch("kis", "2026-08-29", "2026-08-29", [("2026-08-29", "N")])
        path.write_bytes(path.read_bytes().replace(b"fixture", b"altered"))
        self.assertEqual(self.run_inventory(code=2)["error_code"], "CALENDAR_HASH_MISMATCH")

    def test_fails_closed_on_conflict_even_outside_requested_range(self):
        self.batch("kis", "2026-08-29", "2026-08-29",
                   [("2026-08-29", "N"), ("2026-09-06", "N")])
        self.batch("kis-calendar", "2026-09-06", "2026-09-06", [("2026-09-06", "Y")])
        self.assertEqual(self.run_inventory(code=2)["error_code"], "CALENDAR_CONFLICT")

    def test_fails_closed_on_copy_without_committed_original(self):
        dates = [("2026-08-29", "N"), ("2026-08-30", "N")]
        _, original = self.batch("kis-calendar", "2026-08-29", "2026-08-29", dates, committed=False)
        self.batch("kis", "2026-08-30", "2026-08-29", dates, copy=original)
        self.assertEqual(self.run_inventory(code=2)["error_code"], "INVALID_CALENDAR_COPY")

    def test_other_entitlement_cannot_resolve_a_copy_or_supply_coverage(self):
        dates = [("2026-08-29", "N")]
        _, original = self.batch("kis-calendar", "2026-08-29", "2026-08-29", dates,
                                 entitlement="fixture:other")
        self.assertEqual(self.run_inventory(code=2)["error_code"], "NO_MATCHING_CALENDAR_ORIGINALS")
        self.batch("kis", "2026-08-30", "2026-08-29", dates, copy=original)
        self.assertEqual(self.run_inventory(code=2)["error_code"], "INVALID_CALENDAR_COPY")

    def test_legacy_cache_must_match_original_file_name(self):
        dates = [("2026-08-29", "N")]
        self.batch("kis", "2026-08-29", "2026-08-29", dates, file_name="calendar.json")
        self.batch("kis", "2026-08-30", "2026-08-29", dates)
        self.assertEqual(self.run_inventory(code=2)["error_code"], "INVALID_LEGACY_CALENDAR_CACHE")

    def test_commit_lock_busy_and_symlink_sources_are_rejected(self):
        with (self.scope("kis") / "commit.lock").open("rb") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.assertEqual(self.run_inventory(code=2)["error_code"], "RAW_COMMIT_LOCK_BUSY")
        path, _ = self.batch("kis", "2026-08-29", "2026-08-29", [("2026-08-29", "N")])
        renamed = path.with_name("renamed.json")
        path.rename(renamed)
        path.symlink_to(renamed)
        self.assertEqual(self.run_inventory(code=2)["error_code"], "RAW_INVENTORY_UNAVAILABLE")


if __name__ == "__main__":
    unittest.main()
