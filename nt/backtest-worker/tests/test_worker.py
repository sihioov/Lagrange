"""End-to-end worker tests (Todo 20): one isolated NT run, fully normalized.

Builds a deterministic synthetic dataset (phase-0 generator, seed 42), runs
the worker end-to-end against it (temp run dir, isolated child, read-only
dataset, network disabled), and verifies: SUCCEEDED outcome, the 13-section
common model, 9 Parquet artifacts + result.json + manifest.json, byte-identical
dataset after the run, temp run-dir cleanup, and the structured status output.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
import sys
import tempfile
import time
import uuid
from pathlib import Path

import pytest

from helpers import child_env

WORKER_ROOT = Path(__file__).resolve().parent.parent
REPO_ROOT = WORKER_ROOT.parent.parent
PHASE0_DIR = REPO_ROOT / "tests" / "golden" / "phase0"

sys.path.insert(0, str(WORKER_ROOT))

from backtest_worker.worker import Worker  # noqa: E402

EXPECTED_SECTIONS = {
    "summary", "equity", "drawdown", "monthly_returns", "orders", "fills", "positions",
    "cash", "fees", "benchmark", "metrics", "warnings", "provenance",
}


def _load_phase0():
    synth_spec = importlib.util.spec_from_file_location("phase0_synth", PHASE0_DIR / "synth_data.py")
    synth = importlib.util.module_from_spec(synth_spec)
    synth_spec.loader.exec_module(synth)
    runner_spec = importlib.util.spec_from_file_location("phase0_runner", PHASE0_DIR / "runner.py")
    runner = importlib.util.module_from_spec(runner_spec)
    runner_spec.loader.exec_module(runner)
    return synth, runner


def build_dataset(data_root: Path) -> Path:
    synth, runner = _load_phase0()
    rows = synth.generate_curated_rows()
    runner.phase0_dataset.materialize_curated_zone(
        rows, data_root / "curated", version=synth.CURATED_VERSION
    )
    return data_root


def hash_tree(root: Path) -> str:
    digest = hashlib.sha256()
    for path in sorted(root.rglob("*")):
        if path.is_file():
            digest.update(path.relative_to(root).as_posix().encode("utf-8"))
            digest.update(path.read_bytes())
    return digest.hexdigest()


def make_request(dataset: Path, run_id: str | None = None) -> dict:
    return {
        "run_id": run_id or str(uuid.uuid4()),
        "owner_user_id": "123e4567-e89b-12d3-a456-426614174001",
        "job_id": "123e4567-e89b-12d3-a456-426614174002",
        "strategy_path": "ma200_trend:MA200Trend",
        "strategy_config": {
            "ma_period": 200,
            "slippage_bps": 10,
            "lot_size": 100,
            "initial_cash": "100000000",
            "strategy_version": "1.0.0",
            "probe_future_fields": False,
        },
        "strategy_id": "ma200-trend",
        "strategy_version": "1.0.0",
        "dataset_version": "kr-etf-daily-phase0-v2",
        "dataset_path": str(dataset),
        "engine_version": "1.231.0",
        "code_commit": "0123456789abcdef0123456789abcdef01234567",
        "random_seed": 42,
        "timezone": "Asia/Seoul",
        "currency": "KRW",
        "config_sha256": "sha256:" + "a" * 64,
        "slippage_bps": 10,
        "initial_cash": "100000000",
        "limits": {
            "memory_bytes": 2 * 1024 * 1024 * 1024,
            "cpu_seconds": None,
            "wall_seconds": 300,
            "active_processes": 1,
            "network_disabled": True,
        },
        "readonly_mounts": [],
    }


def test_v2_decimal_prices_reach_quote_materialization_as_raw_scale4(tmp_path):
    from backtest_worker.simulate import _read_curated_rows

    dataset = build_dataset(tmp_path / "dataset")
    rows = _read_curated_rows(dataset / "curated")
    first = next(
        row
        for row in rows
        if row["instrument_id"] == "069500.KRX"
        and row["trading_date"] == "2020-01-20"
    )

    assert first["open"] == 101_500_000
    assert first["close"] == 102_500_000


def test_end_to_end_isolated_run_normalizes_results(tmp_path):
    dataset = build_dataset(tmp_path / "dataset")
    before = hash_tree(dataset)

    scratch = tmp_path / "scratch"
    scratch.mkdir()
    output_dir = tmp_path / "artifacts"
    status_path = tmp_path / "run_status.json"
    run_id = str(uuid.uuid4())

    started = time.time()
    outcome = Worker(scratch=scratch).run(make_request(dataset, run_id), output_dir, status_path)
    wall = time.time() - started

    assert outcome.state == "SUCCEEDED", f"worker failed: {outcome.error}"
    assert outcome.run_id == run_id
    assert outcome.isolation["backend"] == "windows-job-object" or outcome.isolation["backend"].startswith("posix")
    assert outcome.isolation["active_processes"] == 1
    assert outcome.isolation["network_disabled"] is True
    assert str(dataset) in outcome.isolation["readonly_mounts"]

    assert hash_tree(dataset) == before, "the read-only dataset must be byte-identical after the run"
    assert len(list(scratch.glob("lagrange-run-*"))) == 0, "temp run directory must be cleaned up"

    result = json.loads((output_dir / "result.json").read_text(encoding="utf-8"))
    assert set(result.keys()) == EXPECTED_SECTIONS
    assert result["provenance"]["strategy_id"] == "ma200-trend"
    assert result["provenance"]["engine"] == "nautilustrader"
    assert result["summary"]["currency"] == "KRW"
    assert result["summary"]["initial_equity"] == {"amount": "100000000.0000", "currency": "KRW"}
    assert result["equity"][0]["ts"].endswith("T00:00:00Z")

    for section in (
        "equity", "drawdown", "monthly_returns", "orders", "fills",
        "positions", "cash", "fees", "benchmark",
    ):
        assert (output_dir / f"{section}.parquet").exists(), f"missing {section}.parquet"

    manifest = json.loads((output_dir / "manifest.json").read_text(encoding="utf-8"))
    assert manifest["run"]["id"] == run_id
    assert manifest["run"]["status"] == "SUCCEEDED"
    assert manifest["run"]["strategy_id"] == "ma200-trend"
    assert len(manifest["artifacts"]) == 9
    for artifact in manifest["artifacts"]:
        assert len(artifact["sha256"]) == 64

    status = json.loads(status_path.read_text(encoding="utf-8"))
    assert status["state"] == "SUCCEEDED"
    assert status["run_id"] == run_id
    assert status["provenance"]["strategy_id"] == "ma200-trend"
    assert isinstance(status["process"]["child_pid"], int)
    assert status["isolation"]["backend"]
    assert wall < 300, f"end-to-end run too slow: {wall}s"
