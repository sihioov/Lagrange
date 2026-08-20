from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import pyarrow.parquet as pq

ROOT = Path(__file__).resolve().parents[2]
PHASE0 = ROOT / "tests" / "golden" / "phase0"
sys.path.insert(0, str(PHASE0))

import phase0_dataset  # noqa: E402
import synth_data  # noqa: E402

EXPECTED = {
    instrument: synth_data.SESSIONS for instrument in synth_data.INSTRUMENTS
}


def destination(value: str) -> Path:
    path = Path(value)
    resolved = (ROOT / path).resolve() if not path.is_absolute() else path.resolve()
    try:
        relative = resolved.relative_to(ROOT)
    except ValueError as exc:
        raise ValueError("destination must be inside repository") from exc
    if relative == Path("."):
        raise ValueError("destination must not be repository root")
    return resolved


def prepare(root: Path) -> dict[str, object]:
    if root.exists() and any(root.iterdir()):
        raise ValueError("destination must be absent or empty")
    root.mkdir(parents=True, exist_ok=True)
    rows = synth_data.generate_curated_rows()
    phase0_dataset.materialize_curated_zone(
        rows, root, version=synth_data.CURATED_VERSION
    )
    counts: dict[str, int] = {}
    paths = sorted(
        root.glob(
            "curated/bars/market=kr/"
            f"symbol=*/year=*/version={synth_data.CURATED_VERSION}/bars.parquet"
        )
    )
    for path in paths:
        table = pq.read_table(path, columns=["instrument_id"])
        for value in table.column("instrument_id").to_pylist():
            counts[value] = counts.get(value, 0) + 1
    if counts != EXPECTED or len(paths) != 6:
        raise RuntimeError(
            f"Phase 0 validation failed: partitions={len(paths)}, sessions={counts}"
        )
    return {
        "root": str(root.relative_to(ROOT)),
        "dataset_version": synth_data.DATA_VERSION,
        "curated_version": synth_data.CURATED_VERSION,
        "sessions": counts,
        "total_bars": sum(counts.values()),
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description="materialize deterministic Phase 0 Parquet data"
    )
    parser.add_argument("--root", required=True)
    args = parser.parse_args()
    try:
        print(json.dumps(prepare(destination(args.root)), sort_keys=True))
        return 0
    except Exception as exc:
        print(f"PREPARE_PHASE0_ERROR: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
