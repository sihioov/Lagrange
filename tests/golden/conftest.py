"""Pytest scaffolding for the golden-manifest harness (Todo 6).

Exposes a hermetic `golden_tree` fixture: a self-contained golden tree
(tmp/tree/golden + tmp/tree/fixtures) that mirrors the committed layout under
tests/fixtures and tests/golden but with minimal synthetic content, so the
harness tests never depend on committed fixture files and can run in any
checkout.  The committed manifest is exercised separately by
test_golden_cli.py::test_committed_manifest_verifies.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPTS_GOLDEN = REPO_ROOT / "scripts" / "golden"
if str(SCRIPTS_GOLDEN) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_GOLDEN))

GOLDEN_PY = SCRIPTS_GOLDEN / "golden.py"


def _write_json(path: Path, obj: object) -> Path:
    path.write_text(
        json.dumps(obj, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    return path


@pytest.fixture
def golden_tree(tmp_path: Path) -> Path:
    """Build a minimal synthetic golden tree; returns the tree root dir."""
    root = tmp_path / "tree"
    golden = root / "golden"
    outputs = golden / "outputs" / "2020-01-31"
    fixtures = root / "fixtures" / "kr-etf" / "2020-01-31"
    variants = root / "fixtures" / "kr-etf" / "variants"
    for d in (outputs, fixtures, variants / "corrupt", variants / "missing", variants / "split-dividend"):
        d.mkdir(parents=True, exist_ok=True)

    # ---- input fixtures (synthetic, minimal but schema-shaped) ----
    bars = {
        "dataset_id": "kr-etf-daily-2020-01-31",
        "schema_version": 1,
        "currency": "KRW",
        "instruments": [{"symbol": "069500.KRX", "lot_size": 100, "currency": "KRW"}],
        "bars": [
            {"instrument": "069500.KRX", "date": "2020-01-31",
             "open": 10200, "high": 10300, "low": 10150, "close": 10250,
             "volume": 1000000, "value": 10250000000},
            {"instrument": "069500.KRX", "date": "2020-02-03",
             "open": 10300, "high": 10400, "low": 10280, "close": 10380,
             "volume": 1100000, "value": 11418000000},
        ],
    }
    _write_json(fixtures / "bars.json", bars)
    _write_json(fixtures / "dataset.json", {
        "dataset_id": "kr-etf-daily-2020-01-31", "version": "1.0.0",
        "source": "synthetic", "rights": "SYNTHETIC_ONLY",
    })
    _write_json(fixtures / "calendar.json", {
        "timezone": "Asia/Seoul",
        "sessions": [{"date": "2020-01-31"}, {"date": "2020-02-03"}],
        "next_session_of": {"2020-01-31": "2020-02-03"},
    })
    _write_json(fixtures / "corporate-actions.json", {"schema_version": 1, "actions": []})
    _write_json(fixtures / "session-semantics.json", {
        "signal_session": "2020-01-31",
        "next_krx_session": "2020-02-03",
        "execution_price_source": "next_krx_session.open",
    })

    # ---- approved golden outputs (synthetic) ----
    _write_json(outputs / "recommendation.json", {
        "schema_version": 1, "as_of_date": "2020-01-31", "effective_date": "2020-02-03",
        "target_portfolio": [{"instrument": "069500.KRX", "target_weight": "0.45"}],
    })
    _write_json(outputs / "orders.json", {
        "schema_version": 1,
        "orders": [{"order_id": "ord-0001", "instrument": "069500.KRX", "side": "BUY", "quantity": 400}],
    })
    _write_json(outputs / "fills.json", {
        "schema_version": 1,
        "fills": [{"fill_id": "fill-0001", "order_id": "ord-0001", "instrument": "069500.KRX",
                   "side": "BUY", "quantity": 400, "price": "10300.00",
                   "ts": "2020-02-03T00:00:00.000Z", "source": "NEXT_SESSION_OPEN",
                   "slippage_bps": 0}],
    })
    _write_json(outputs / "equity.json", {
        "schema_version": 1,
        "points": [{"date": "2020-01-31", "equity": "10000000.00"},
                   {"date": "2020-02-03", "equity": "10056000.00"}],
    })
    _write_json(outputs / "fees.json", {"schema_version": 1, "total_fees": "3000.00", "items": []})
    _write_json(outputs / "metrics.json", {"schema_version": 1, "total_return_pct": "0.56", "finite": True})
    _write_json(outputs / "provenance.json", {
        "schema_version": 1, "engine": "nautilustrader", "engine_version": "1.231.0",
        "random_seed": 42, "timezone": "Asia/Seoul",
    })

    # ---- variants ----
    (variants / "corrupt" / "corrupt_bars.bin").write_bytes(
        b"PAR1" + b"\x00" * 64 + b"\xff\xfe\xfd\x00" + b"PAR1")
    missing = dict(bars)
    missing["dataset_id"] = "kr-etf-daily-2020-01-31-missing"
    missing["bars"] = [b for b in bars["bars"] if b["date"] != "2020-01-31"]
    missing["missing"] = {"instrument": "069500.KRX", "date": "2020-01-31"}
    _write_json(variants / "missing" / "missing_bars.json", missing)
    _write_json(variants / "split-dividend" / "actions.json", {
        "schema_version": 1,
        "actions": [{"instrument": "069500.KRX", "type": "split", "ratio": "2:1",
                     "ex_date": "2020-02-03"}],
    })

    # ---- golden generation config ----
    config = {
        "golden_id": "kr-etf-2020-01-31-test",
        "manifest_version": "1",
        "versions": {
            "data": {"id": "kr-etf-daily-2020-01-31", "version": "1.0.0", "source": "synthetic"},
            "strategy": {"id": "golden-baseline", "version": "1.0.0"},
            "engine": {"name": "nautilustrader", "version": "1.231.0"},
            "config": {"id": "golden-config-v1"},
            "seed": 42,
            "timezone": "Asia/Seoul",
        },
        "fixtures": [
            {"path": "../fixtures/kr-etf/2020-01-31/dataset.json", "category": "data-dataset"},
            {"path": "../fixtures/kr-etf/2020-01-31/bars.json", "category": "data-bars"},
            {"path": "../fixtures/kr-etf/2020-01-31/calendar.json", "category": "data-calendar"},
            {"path": "../fixtures/kr-etf/2020-01-31/corporate-actions.json", "category": "data-corporate-actions"},
            {"path": "../fixtures/kr-etf/2020-01-31/session-semantics.json", "category": "data-session-semantics"},
            {"path": "../fixtures/kr-etf/variants/corrupt/corrupt_bars.bin", "category": "data-corrupt-bytes"},
            {"path": "../fixtures/kr-etf/variants/missing/missing_bars.json", "category": "data-missing-bars"},
            {"path": "../fixtures/kr-etf/variants/split-dividend/actions.json", "category": "data-split-dividend"},
        ],
        "artifacts": [
            {"path": "outputs/2020-01-31/recommendation.json", "category": "recommendation"},
            {"path": "outputs/2020-01-31/orders.json", "category": "order"},
            {"path": "outputs/2020-01-31/fills.json", "category": "fill"},
            {"path": "outputs/2020-01-31/equity.json", "category": "equity"},
            {"path": "outputs/2020-01-31/fees.json", "category": "fee"},
            {"path": "outputs/2020-01-31/metrics.json", "category": "metric"},
            {"path": "outputs/2020-01-31/provenance.json", "category": "provenance"},
        ],
    }
    _write_json(golden / "golden.json", config)
    return root
