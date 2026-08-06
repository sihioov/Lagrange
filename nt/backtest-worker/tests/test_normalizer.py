"""Red tests (Todo 20): normalizing NT raw results into the common model.

Every normalizer invariant from design §6.10 and plan Todo 20 is tested here:
NaN/Infinity rejection, date-regression rejection, ledger-mismatch rejection
(cash + positions != equity, fills vs positions, cash vs fills+fees), refusal to
publish after an integrity failure, the exact 13-section common-model shape,
Parquet materialization of the 9 large arrays, and the provenance/status
output shape.
"""
from __future__ import annotations

import json
import math
import sys
from pathlib import Path

import pytest

from helpers import child_env

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))


def make_raw() -> dict:
    return {
        "orders": {
            "orders": [
                {
                    "order_id": "O-1",
                    "client_order_id": "C-1",
                    "instrument": "069500.KRX",
                    "side": "BUY",
                    "quantity": 3300,
                    "order_type": "MARKET",
                    "signal_date": "2020-01-01",
                    "created_ts": "2020-01-01T06:30:00Z",
                    "execution_ts_target": "2020-01-02T00:00:00Z",
                    "state": "FILLED",
                }
            ]
        },
        "fills": {
            "fills": [
                {
                    "fill_id": "F-1",
                    "order_id": "O-1",
                    "client_order_id": "C-1",
                    "instrument": "069500.KRX",
                    "side": "BUY",
                    "quantity": 3300,
                    "price_raw": 101060960,
                    "ts": "2020-01-02T00:00:00Z",
                    "commission_raw": 0,
                    "tax_raw": 0,
                }
            ]
        },
        "equity": {
            "initial_cash_raw": 1000000000000,
            "points": [
                {
                    "date": "2020-01-02",
                    "cash_raw": 666498832000,
                    "positions_value_raw": 333501168000,
                    "equity_raw": 1000000000000,
                },
                {
                    "date": "2020-12-31",
                    "cash_raw": 666498832000,
                    "positions_value_raw": 344110768000,
                    "equity_raw": 1010609600000,
                },
            ],
        },
        "positions": {
            "positions": [
                {"date": "2020-01-02", "instrument": "069500.KRX", "quantity": 3300}
            ]
        },
        "fees": {"cost_profile": {}, "total_fees_raw": 0, "items": []},
        "benchmark": {
            "points": [
                {"date": "2020-01-02", "value_raw": 1000000000000},
                {"date": "2020-12-31", "value_raw": 1020000000000},
            ]
        },
        "provenance": {
            "engine": "nautilustrader",
            "engine_version": "1.231.0",
            "strategy_id": "ma200_trend",
            "strategy_version": "1.0.0",
            "dataset_version": "kr-etf-daily-20260804.1",
            "config_hash": "sha256:abcd1234",
            "code_commit": "abcdef1234567",
            "random_seed": 42,
            "timezone": "Asia/Seoul",
        },
        "metrics": {"total_return_pct": "1.060960", "max_drawdown_pct": "0.000000"},
    }


def normalize(raw: dict):
    from backtest_worker.normalizer import Normalizer
    from backtest_worker.raw import RawResult

    return Normalizer().normalize(RawResult.from_dict(raw))


@pytest.fixture()
def valid_raw() -> dict:
    return make_raw()


def test_common_model_has_exactly_13_sections(valid_raw):
    result = normalize(valid_raw)
    expected = {
        "summary", "equity", "drawdown", "monthly_returns", "orders", "fills", "positions",
        "cash", "fees", "benchmark", "metrics", "warnings", "provenance",
    }
    assert set(result.keys()) == expected, f"unexpected sections: {set(result.keys()) - expected}"


def test_money_is_scale4_decimal_strings(valid_raw):
    result = normalize(valid_raw)
    assert result["summary"]["initial_equity"] == "100000000.0000"
    assert result["summary"]["final_equity"] == "101060960.0000"
    assert result["equity"][0]["equity"] == "100000000.0000"
    assert result["cash"][0]["cash"] == "66649883.2000"
    assert result["fills"][0]["price"] == "10106.0960"
    assert result["benchmark"][1]["value"] == "102000000.0000"


def test_total_return_and_metrics_are_computed_finite(valid_raw):
    result = normalize(valid_raw)
    assert result["summary"]["total_return"] == pytest.approx(0.0106096, abs=1e-9)
    for key, value in result["metrics"].items():
        assert math.isfinite(value), f"metric {key} is not finite: {value}"
    assert result["metrics"]["total_return"] == pytest.approx(0.0106096, abs=1e-9)
    assert result["metrics"]["benchmark_return"] == pytest.approx(0.02, abs=1e-9)


def test_provenance_shape_matches_design(valid_raw):
    result = normalize(valid_raw)
    for key in (
        "engine", "engine_version", "strategy_id", "strategy_version", "dataset_version",
        "config_hash", "code_commit", "random_seed", "timezone",
    ):
        assert key in result["provenance"], f"provenance missing {key}"
    assert result["provenance"]["engine"] == "nautilustrader"
    assert result["provenance"]["timezone"] == "Asia/Seoul"
    assert result["provenance"]["random_seed"] == 42


def test_nan_in_raw_is_rejected(valid_raw):
    valid_raw["fills"]["fills"][0]["quantity"] = float("nan")
    with pytest.raises(Exception) as exc:
        normalize(valid_raw)
    assert "finite" in str(exc.value).lower() or "nan" in str(exc.value).lower()


def test_infinity_in_raw_is_rejected(valid_raw):
    valid_raw["equity"]["points"][0]["equity_raw"] = float("inf")
    with pytest.raises(Exception) as exc:
        normalize(valid_raw)
    assert "finite" in str(exc.value).lower() or "infinity" in str(exc.value).lower()


def test_date_regression_is_rejected(valid_raw):
    valid_raw["equity"]["points"][0]["date"] = "2020-12-31"
    with pytest.raises(Exception) as exc:
        normalize(valid_raw)
    assert "regression" in str(exc.value).lower()


def test_ledger_mismatch_cash_plus_positions_neq_equity_is_rejected(valid_raw):
    valid_raw["equity"]["points"][1]["equity_raw"] = 1010609600000 + 5000000000
    with pytest.raises(Exception) as exc:
        normalize(valid_raw)
    assert "ledger" in str(exc.value).lower()


def test_ledger_mismatch_fills_vs_positions_is_rejected(valid_raw):
    valid_raw["positions"]["positions"][0]["quantity"] = 3200
    with pytest.raises(Exception) as exc:
        normalize(valid_raw)
    assert "ledger" in str(exc.value).lower()


def test_ledger_mismatch_cash_vs_fills_is_rejected(valid_raw):
    valid_raw["equity"]["points"][0]["cash_raw"] = 666498832000 + 10000000
    with pytest.raises(Exception) as exc:
        normalize(valid_raw)
    assert "ledger" in str(exc.value).lower()


def test_publication_refused_after_integrity_failure(valid_raw):
    from backtest_worker.normalizer import IntegrityGate, Normalizer
    from backtest_worker.raw import RawResult

    valid_raw["equity"]["points"][0]["date"] = "2020-12-31"
    result = Normalizer().normalize(RawResult.from_dict(valid_raw), validate=False)
    gate = IntegrityGate()
    assert gate.validate(result) is not None, "the broken result must fail validation"
    with pytest.raises(Exception) as exc:
        gate.publish()
    assert "publish" in str(exc.value).lower()


def test_publication_allowed_after_valid_validation(valid_raw):
    from backtest_worker.normalizer import IntegrityGate

    result = normalize(valid_raw)
    gate = IntegrityGate()
    assert gate.validate(result) is None
    gate.publish()


def test_parquet_artifacts_written_for_all_9_arrays(valid_raw, tmp_path):
    from backtest_worker.normalizer import write_parquet_artifacts

    result = normalize(valid_raw)
    artifacts = write_parquet_artifacts(result, tmp_path)
    import pyarrow.parquet as pq

    for section in (
        "equity", "drawdown", "monthly_returns", "orders", "fills",
        "positions", "cash", "fees", "benchmark",
    ):
        path = tmp_path / f"{section}.parquet"
        assert path.exists(), f"missing parquet for {section}"
        table = pq.read_table(path)
        assert table.num_rows == len(result[section])
        assert artifacts[section]["sha256"] and artifacts[section]["row_count"] == table.num_rows


def test_manifest_json_built_from_normalized_result(valid_raw, tmp_path):
    from backtest_worker.publish import build_manifest

    result = normalize(valid_raw)
    manifest = build_manifest(
        result,
        run_id="123e4567-e89b-12d3-a456-426614174000",
        owner_user_id="123e4567-e89b-12d3-a456-426614174001",
        job_id="123e4567-e89b-12d3-a456-426614174002",
        run_dir=tmp_path,
        status="SUCCEEDED",
    )
    assert manifest["run"]["id"] == "123e4567-e89b-12d3-a456-426614174000"
    assert manifest["run"]["status"] == "SUCCEEDED"
    assert manifest["run"]["strategy_id"] == "ma200_trend"
    assert len(manifest["artifacts"]) == 9
    for artifact in manifest["artifacts"]:
        assert artifact["sha256"]
        assert artifact["row_count"] >= 0
        assert artifact["artifact_type"] in {
            "EQUITY_CURVE", "DRAWDOWN_CURVE", "MONTHLY_RETURNS", "ORDERS", "FILLS",
            "POSITIONS", "CASH_LEDGER", "FEES", "BENCHMARK",
        }
