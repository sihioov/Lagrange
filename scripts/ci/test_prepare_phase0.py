from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import pyarrow.parquet as pq

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "ci" / "prepare_phase0.py"
EXPECTED = {"069500.KRX": 260, "229200.KRX": 260, "114260.KRX": 260}


def run_prepare(root: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--root", str(root)],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )


class PreparePhase0Tests(unittest.TestCase):
    def test_materializes_the_exact_phase0_partition_contract(self) -> None:
        with tempfile.TemporaryDirectory(prefix=".phase0-test-", dir=ROOT) as tmp:
            root = Path(tmp) / "phase0"
            proc = run_prepare(root)
            self.assertEqual(proc.returncode, 0, proc.stderr)
            summary = json.loads(proc.stdout)
            self.assertEqual(summary["sessions"], EXPECTED)
            self.assertEqual(summary["total_bars"], 780)
            paths = sorted(
                root.glob(
                    "curated/curated/bars/market=kr/"
                    "symbol=*/year=*/version=1/bars.parquet"
                )
            )
            counts: dict[str, int] = {}
            for path in paths:
                table = pq.read_table(path, columns=["instrument_id"])
                for value in table.column("instrument_id").to_pylist():
                    counts[value] = counts.get(value, 0) + 1
            self.assertEqual(counts, EXPECTED)
            self.assertEqual(len(paths), 6)

    def test_refuses_a_nonempty_destination(self) -> None:
        with tempfile.TemporaryDirectory(prefix=".phase0-test-", dir=ROOT) as tmp:
            root = Path(tmp) / "phase0"
            root.mkdir()
            (root / "stale").write_text("stale", encoding="utf-8")
            proc = run_prepare(root)
            self.assertNotEqual(proc.returncode, 0)
            self.assertIn("destination must be absent or empty", proc.stderr)

    def test_refuses_a_destination_outside_the_repository(self) -> None:
        with tempfile.TemporaryDirectory(prefix="phase0-outside-") as tmp:
            proc = run_prepare(Path(tmp) / "phase0")
            self.assertNotEqual(proc.returncode, 0)
            self.assertIn("destination must be inside repository", proc.stderr)


if __name__ == "__main__":
    unittest.main()
