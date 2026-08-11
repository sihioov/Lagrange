from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from datetime import date
from decimal import Decimal
from pathlib import Path

import pyarrow as pa
import pyarrow.compute as pc
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
            self.assertEqual(summary["dataset_version"], "kr-etf-daily-phase0-v2")
            self.assertEqual(summary["curated_version"], 2)
            self.assertEqual(summary["sessions"], EXPECTED)
            self.assertEqual(summary["total_bars"], 780)
            paths = sorted(
                root.glob(
                    "curated/curated/bars/market=kr/"
                    "symbol=*/year=*/version=*/bars.parquet"
                )
            )
            self.assertEqual({path.parent.name for path in paths}, {"version=2"})
            counts: dict[str, int] = {}
            tables: list[pa.Table] = []
            for path in paths:
                table = pq.read_table(
                    path,
                    columns=["instrument_id", "trading_date", "open", "high", "low", "close"],
                )
                tables.append(table)
                for value in table.column("instrument_id").to_pylist():
                    counts[value] = counts.get(value, 0) + 1
            self.assertEqual(counts, EXPECTED)
            self.assertEqual(len(paths), 6)

            bars = pa.concat_tables(tables)
            self.assertEqual(bars.num_rows, 780)
            for row in bars.to_pylist():
                self.assertGreater(row["open"], 0)
                self.assertGreater(row["close"], 0)
                self.assertLessEqual(row["low"], min(row["open"], row["close"]))
                self.assertGreaterEqual(row["high"], max(row["open"], row["close"]))

            target = bars.filter(
                pc.and_(
                    pc.equal(bars["instrument_id"], pa.scalar("069500.KRX")),
                    pc.equal(bars["trading_date"], pa.scalar(date(2020, 1, 20), type=pa.date32())),
                )
            )
            self.assertEqual(target.num_rows, 1)
            target_open = target["open"][0].as_py()
            self.assertEqual(target_open, Decimal("10150.0000"))
            self.assertNotEqual(target_open, Decimal("101500000.0000"))

            adjusted_paths = [path.with_name("adjusted_bars.parquet") for path in paths]
            for path in adjusted_paths:
                factors = pq.read_table(path, columns=["adjustment_factor"])[
                    "adjustment_factor"
                ].to_pylist()
                self.assertTrue(factors)
                self.assertTrue(all(value == Decimal("1.00000000") for value in factors))

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
