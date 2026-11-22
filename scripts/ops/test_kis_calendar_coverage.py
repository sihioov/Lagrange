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

    @staticmethod
    def capacity_record(batch_number, *, provider="kis", partition_date="2026-09-01",
                        file_count=0, file_names=None, extra=None):
        names = file_names if file_names is not None else [
            f"source-{index:05d}.json" for index in range(file_count)
        ]
        record = {
            "batch_id": str(uuid.UUID(int=batch_number)),
            "provider": provider,
            "market": "kr",
            "date": partition_date,
            "files": [{"file_name": name} for name in names],
        }
        if extra is not None:
            record["extra"] = extra
        return record

    def write_capacity_wire(self, provider, wire):
        (self.scope(provider) / "manifest.jsonl").write_bytes(wire)

    def write_capacity_records(self, provider, records, *, terminated=True):
        lines = [json.dumps(record, sort_keys=True, separators=(",", ":")).encode()
                 for record in records]
        wire = b"\n".join(lines)
        if terminated and lines:
            wire += b"\n"
        self.write_capacity_wire(provider, wire)

    def run_capacity(self, *, code=0):
        result = subprocess.run([
            sys.executable, "-I", "-S", "-B", str(SCRIPT), "--data-root", str(self.root),
            "--capacity-only",
        ], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, code, result.stdout + result.stderr)
        self.assertEqual(result.stderr, "")
        self.assertNotIn(ENTITLEMENT, result.stdout)
        self.assertNotIn("fixture body must stay private", result.stdout)
        self.assertEqual(len(result.stdout.splitlines()), 1)
        return json.loads(result.stdout)

    @staticmethod
    def metrics_for(result, provider="kis"):
        return next(scope["metrics"] for scope in result["scopes"]
                    if scope["provider"] == provider)

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
            self.assertEqual(self.run_capacity(code=2)["error_code"], "RAW_COMMIT_LOCK_BUSY")
        path, _ = self.batch("kis", "2026-08-29", "2026-08-29", [("2026-08-29", "N")])
        renamed = path.with_name("renamed.json")
        path.rename(renamed)
        path.symlink_to(renamed)
        self.assertEqual(self.run_inventory(code=2)["error_code"], "RAW_INVENTORY_UNAVAILABLE")
        self.assertEqual(self.run_capacity()["status"], "capacity_projection")
        manifest = self.scope("kis") / "manifest.jsonl"
        moved_manifest = manifest.with_name("manifest-copy.jsonl")
        manifest.rename(moved_manifest)
        manifest.symlink_to(moved_manifest)
        self.assertEqual(self.run_capacity(code=2)["error_code"], "RAW_INVENTORY_UNAVAILABLE")

    def test_capacity_empty_manifest_and_runtime_path_projection(self):
        result = self.run_capacity()
        self.assertEqual(result["status"], "capacity_projection")
        self.assertEqual(result["measurement"], "committed_manifest_metadata_only")
        self.assertEqual(result["descriptor_projection_assumptions"], {
            "live_data_root_components": 1,
            "paper_archive_data_root_components": 4,
            "paper_archive_coordination_components": 4,
        })
        for provider in ("kis", "kis-calendar"):
            metrics = self.metrics_for(result, provider)
            self.assertEqual(metrics["manifest_records"]["observed"], 0)
            self.assertEqual(metrics["batches"]["observed"], 0)
            self.assertEqual(metrics["files"]["observed"], 0)
            self.assertEqual(metrics["distinct_dates"], 0)
            self.assertEqual(metrics["manifest_bytes"]["observed"], 0)
            self.assertEqual(metrics["live_descriptors_projected"]["observed"], 8)
            self.assertEqual(metrics["paper_archive_descriptors_projected"]["observed"], 15)
        self.assertNotIn("ready", result)
        self.assertEqual(set(result["not_assessed"]), {
            "provider body presence, size, or hash",
            "native full-schema or byte verification",
            "entitlement rights or runtime readiness",
        })

    def test_capacity_counts_supplier_equality_and_accepts_blank_lines(self):
        records = [self.capacity_record(index + 1, file_count=64) for index in range(128)]
        wire = b"\t \n" + b"\n".join(
            json.dumps(record, sort_keys=True, separators=(",", ":")).encode()
            for record in records
        )
        self.write_capacity_wire("kis", wire)
        metrics = self.metrics_for(self.run_capacity())
        self.assertEqual(metrics["manifest_records"], {
            "observed": 128, "inclusive_limit": 128, "within_limit": True,
        })
        self.assertEqual(metrics["batches"], {
            "observed": 128, "inclusive_limit": 128, "within_limit": True,
        })
        self.assertEqual(metrics["files"], {
            "observed": 8192, "inclusive_limit": 8192, "within_limit": True,
        })
        self.assertEqual(metrics["largest_batch_files"]["observed"], 64)
        self.assertEqual(metrics["distinct_dates"], 1)
        self.assertEqual(metrics["live_descriptors_projected"], {
            "observed": 8459, "inclusive_limit": 16384, "within_limit": True,
        })
        self.assertEqual(metrics["paper_archive_descriptors_projected"], {
            "observed": 8466, "inclusive_limit": 4096, "within_limit": False,
        })

    def test_capacity_reports_record_batch_and_file_overages(self):
        records = [self.capacity_record(index + 1, file_count=64) for index in range(129)]
        self.write_capacity_records("kis", records, terminated=False)
        metrics = self.metrics_for(self.run_capacity())
        self.assertEqual(metrics["manifest_records"]["observed"], 129)
        self.assertFalse(metrics["manifest_records"]["within_limit"])
        self.assertEqual(metrics["batches"]["observed"], 129)
        self.assertFalse(metrics["batches"]["within_limit"])
        self.assertEqual(metrics["files"]["observed"], 8256)
        self.assertFalse(metrics["files"]["within_limit"])

    def test_capacity_counts_semantically_duplicate_rows_once_and_accepts_native_names(self):
        record = self.capacity_record(1, file_names=["source..한국.json"])
        line = json.dumps(record, sort_keys=True, separators=(",", ":")).encode()
        self.write_capacity_wire("kis", b" \t\r\n" + line + b"\n" + line)
        metrics = self.metrics_for(self.run_capacity())
        self.assertEqual(metrics["manifest_records"]["observed"], 2)
        self.assertEqual(metrics["batches"]["observed"], 1)
        self.assertEqual(metrics["files"]["observed"], 1)
        self.assertEqual(metrics["largest_batch_files"]["observed"], 1)

    def test_capacity_rejects_conflicts_duplicate_keys_and_malformed_file_paths(self):
        first = self.capacity_record(1, extra=1)
        second = self.capacity_record(1, extra=True)
        self.write_capacity_records("kis", [first, second])
        self.assertEqual(self.run_capacity(code=2)["error_code"],
                         "INDETERMINATE_DUPLICATE_METADATA")

        self.write_capacity_wire("kis", b"\x0b\n")
        self.assertEqual(self.run_capacity(code=2)["error_code"], "INVALID_MANIFEST")

        self.write_capacity_wire("kis", b'{"batch_id":"x","batch_id":"y"}\n')
        self.assertEqual(self.run_capacity(code=2)["error_code"], "DUPLICATE_JSON_KEY")

        bad_path = self.capacity_record(1, file_names=["../provider-secret.json"])
        self.write_capacity_records("kis", [bad_path])
        self.assertEqual(self.run_capacity(code=2)["error_code"], "INVALID_MANIFEST")

        missing = self.capacity_record(1)
        del missing["date"]
        self.write_capacity_records("kis", [missing])
        self.assertEqual(self.run_capacity(code=2)["error_code"], "INVALID_DATE")

        self.write_capacity_wire("kis", b'{"batch_id":')
        self.assertEqual(self.run_capacity(code=2)["error_code"], "INDETERMINATE_MANIFEST_TAIL")

        self.write_capacity_wire("kis", b"not-json\n")
        self.assertEqual(self.run_capacity(code=2)["error_code"], "INVALID_MANIFEST")

    def test_capacity_parses_both_snapshots_after_releasing_shared_locks(self):
        self.write_capacity_records("kis", [self.capacity_record(1)])
        checker = '''
import fcntl
import importlib.util
from pathlib import Path
import sys

script, root = Path(sys.argv[1]), Path(sys.argv[2])
spec = importlib.util.spec_from_file_location("capacity_coverage_test", script)
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)
original = module.capacity_scope

def check_after_release(provider, wire):
    for name in ("kis", "kis-calendar"):
        lock = root / "raw" / "manifests" / ("provider=" + name) / "market=kr" / "commit.lock"
        with lock.open("rb") as source:
            fcntl.flock(source, fcntl.LOCK_EX | fcntl.LOCK_NB)
            fcntl.flock(source, fcntl.LOCK_UN)
    return original(provider, wire)

module.capacity_scope = check_after_release
module.capacity_inventory(root)
'''
        result = subprocess.run([
            sys.executable, "-I", "-S", "-B", "-c", checker, str(SCRIPT), str(self.root),
        ], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual(result.stderr, "")

    def test_capacity_enforces_largest_batch_and_descriptor_inclusive_boundaries(self):
        self.write_capacity_records("kis", [self.capacity_record(1, file_count=256)])
        metrics = self.metrics_for(self.run_capacity())
        self.assertEqual(metrics["largest_batch_files"], {
            "observed": 256, "inclusive_limit": 256, "within_limit": True,
        })
        self.write_capacity_records("kis", [self.capacity_record(1, file_count=257)])
        metrics = self.metrics_for(self.run_capacity())
        self.assertFalse(metrics["largest_batch_files"]["within_limit"])

        self.write_capacity_records("kis", [self.capacity_record(1, file_count=16371)])
        metrics = self.metrics_for(self.run_capacity())
        self.assertEqual(metrics["live_descriptors_projected"], {
            "observed": 16384, "inclusive_limit": 16384, "within_limit": True,
        })
        self.write_capacity_records("kis", [self.capacity_record(1, file_count=16372)])
        metrics = self.metrics_for(self.run_capacity())
        self.assertEqual(metrics["live_descriptors_projected"]["observed"], 16385)
        self.assertFalse(metrics["live_descriptors_projected"]["within_limit"])

        self.write_capacity_records("kis", [self.capacity_record(1, file_count=4076)])
        metrics = self.metrics_for(self.run_capacity())
        self.assertEqual(metrics["paper_archive_descriptors_projected"], {
            "observed": 4096, "inclusive_limit": 4096, "within_limit": True,
        })
        self.write_capacity_records("kis", [self.capacity_record(1, file_count=4077)])
        metrics = self.metrics_for(self.run_capacity())
        self.assertEqual(metrics["paper_archive_descriptors_projected"]["observed"], 4097)
        self.assertFalse(metrics["paper_archive_descriptors_projected"]["within_limit"])

    def test_capacity_manifest_byte_limit_is_inclusive_and_read_is_bounded(self):
        line = json.dumps(self.capacity_record(1), sort_keys=True, separators=(",", ":")).encode() + b"\n"
        self.write_capacity_wire("kis", line + b" " * (8_388_608 - len(line)))
        metrics = self.metrics_for(self.run_capacity())
        self.assertEqual(metrics["manifest_bytes"], {
            "observed": 8_388_608, "inclusive_limit": 8_388_608, "within_limit": True,
        })
        self.write_capacity_wire("kis", line + b" " * (8_388_609 - len(line)))
        self.assertEqual(self.run_capacity(code=2)["error_code"], "RAW_LIMIT_EXCEEDED")

    def test_capacity_inspection_caps_bound_large_records_and_file_lists(self):
        line = json.dumps(self.capacity_record(1), sort_keys=True, separators=(",", ":")).encode() + b"\n"
        self.write_capacity_wire("kis", line * 16_385)
        self.assertEqual(self.run_capacity(code=2)["error_code"], "CAPACITY_INSPECTION_LIMIT")

        record = self.capacity_record(1, file_count=65_537)
        self.write_capacity_records("kis", [record])
        self.assertEqual(self.run_capacity(code=2)["error_code"], "CAPACITY_INSPECTION_LIMIT")

    def test_capacity_flags_do_not_change_normal_cli_contract(self):
        missing_entitlement = subprocess.run([
            sys.executable, "-I", "-S", "-B", str(SCRIPT), "--data-root", str(self.root),
        ], capture_output=True, text=True, check=False)
        self.assertEqual(missing_entitlement.returncode, 2)
        self.assertEqual(missing_entitlement.stderr, "")
        self.assertEqual(json.loads(missing_entitlement.stdout), {
            "status": "blocked", "error_code": "INVALID_ARGUMENT",
        })
        conflicting_modes = subprocess.run([
            sys.executable, "-I", "-S", "-B", str(SCRIPT), "--data-root", str(self.root),
            "--capacity-only", "--entitlement-sha256", digest(ENTITLEMENT.encode()),
        ], capture_output=True, text=True, check=False)
        self.assertEqual(conflicting_modes.returncode, 2)
        self.assertEqual(conflicting_modes.stderr, "")
        self.assertEqual(json.loads(conflicting_modes.stdout), {
            "status": "blocked", "error_code": "INVALID_ARGUMENT",
        })
        as_of_capacity = subprocess.run([
            sys.executable, "-I", "-S", "-B", str(SCRIPT), "--data-root", str(self.root),
            "--capacity-only", "--through", "2026-09-05",
        ], capture_output=True, text=True, check=False)
        self.assertEqual(as_of_capacity.returncode, 2)
        self.assertEqual(as_of_capacity.stderr, "")
        self.assertEqual(json.loads(as_of_capacity.stdout), {
            "status": "blocked", "error_code": "INVALID_ARGUMENT",
        })


if __name__ == "__main__":
    unittest.main()
