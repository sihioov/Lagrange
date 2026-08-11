"""Contract tests for the isolated recommendation target child."""

from __future__ import annotations

import json
import math
import os
import subprocess
import sys
from pathlib import Path

import pytest

from strategy_helpers import NT_ROOT, STRATEGIES, load_package


MEMBERS = [
    "069500.KRX",
    "102110.KRX",
    "229200.KRX",
    "143850.KRX",
    "133690.KRX",
    "195930.KRX",
    "192090.KRX",
    "148070.KRX",
    "114260.KRX",
    "153130.KRX",
    "132030.KRX",
]
PROVENANCE = {
    "dataset_version_id": "123e4567-e89b-42d3-a456-426614174000",
    "dataset_id": "krx_eod_bars",
    "dataset_version": "phase0-v2",
    "curated_version": 2,
    "dataset_manifest_sha256": "c" * 64,
    "universe_snapshot_id": "sha256:" + "a" * 64,
    "factor_snapshot_hash": "sha256:" + "b" * 64,
}


def request_for(strategy_id: str) -> dict:
    package = load_package(strategy_id)
    parameters = dict(package.DEFAULT_PARAMETERS)
    factors = {member: {} for member in MEMBERS}
    if strategy_id == "trend_following":
        factors["069500.KRX"] = {"trend_50": 110.0, "trend_200": 100.0}
    elif strategy_id == "relative_momentum":
        for index, member in enumerate(MEMBERS):
            factors[member] = {"momentum_12_1": float(index) / 100.0}
    elif strategy_id == "dual_momentum":
        for index, member in enumerate(MEMBERS):
            factors[member] = {"return_12m": float(index) / 100.0}
    elif strategy_id == "inverse_volatility":
        for index, member in enumerate(MEMBERS):
            factors[member] = {"vol_60": 0.1 + float(index) / 100.0}
    return {
        "strategy_id": strategy_id,
        "strategy_version": package.VERSION,
        "parameters": parameters,
        "as_of": "2020-12-30",
        "universe": MEMBERS,
        "factors": factors,
        "provenance": PROVENANCE,
    }


def run_cli(tmp_path: Path, request: object, *, raw: bytes | None = None):
    request_path = tmp_path / "request.json"
    result_path = tmp_path / "result.json"
    status_path = tmp_path / "status.json"
    request_path.write_bytes(
        raw
        if raw is not None
        else json.dumps(request, sort_keys=True, allow_nan=True).encode("utf-8")
    )
    env = os.environ.copy()
    env["PYTHONPATH"] = str(NT_ROOT)
    completed = subprocess.run(
        [
            sys.executable,
            "-m",
            "strategies.recommendation_cli",
            "--request",
            str(request_path),
            "--result",
            str(result_path),
            "--status",
            str(status_path),
        ],
        cwd=NT_ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    result = json.loads(result_path.read_text("utf-8")) if result_path.exists() else None
    status = json.loads(status_path.read_text("utf-8")) if status_path.exists() else None
    return completed, result, status, result_path.read_bytes() if result_path.exists() else None


@pytest.mark.parametrize("strategy_id", STRATEGIES)
def test_all_five_shipped_generators_run_through_closed_contract(tmp_path, strategy_id):
    completed, result, status, _ = run_cli(tmp_path, request_for(strategy_id))
    assert completed.returncode == 0, completed.stderr
    assert status is None
    assert result["strategy_version"] == f"{strategy_id}@1.0.0"
    assert result["as_of"] == "2020-12-30"
    for key, value in PROVENANCE.items():
        assert result[key] == value


def test_identical_request_bytes_produce_byte_identical_canonical_result(tmp_path):
    request = request_for("relative_momentum")
    first = tmp_path / "first"
    second = tmp_path / "second"
    first.mkdir()
    second.mkdir()
    first_run = run_cli(first, request)
    second_run = run_cli(second, request)
    assert first_run[0].returncode == second_run[0].returncode == 0
    assert first_run[3] == second_run[3]
    assert first_run[3].endswith(b"\n")
    assert first_run[3] == (
        json.dumps(
            json.loads(first_run[3]),
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
        + b"\n"
    )


@pytest.mark.parametrize(
    ("mutate", "expected_code"),
    [
        (lambda request: request.update(strategy_id="not_shipped"), "UNKNOWN_STRATEGY"),
        (lambda request: request.update(unexpected=True), "INVALID_REQUEST"),
        (lambda request: request.update(strategy_version="2.0.0"), "UNSUPPORTED_VERSION"),
        (lambda request: request.update(module="os:system"), "INVALID_REQUEST"),
        (lambda request: request.update(as_of="2020-1-2"), "INVALID_REQUEST"),
        (lambda request: request.update(universe=MEMBERS[:-1]), "INVALID_REQUEST"),
    ],
)
def test_invalid_envelope_fails_without_result_or_request_echo(tmp_path, mutate, expected_code):
    request = request_for("buy_and_hold")
    mutate(request)
    completed, result, status, _ = run_cli(tmp_path, request)
    assert completed.returncode != 0
    assert result is None
    assert status["code"] == expected_code
    assert set(status) == {"code", "summary"}
    assert "069500.KRX" not in status["summary"]
    assert len(json.dumps(status).encode("utf-8")) <= 16 * 1024


@pytest.mark.parametrize("bad", [math.nan, math.inf, -math.inf, "0.1", True])
def test_malformed_or_nonfinite_factor_is_rejected(tmp_path, bad):
    request = request_for("relative_momentum")
    request["factors"][MEMBERS[0]]["momentum_12_1"] = bad
    completed, result, status, _ = run_cli(tmp_path, request)
    assert completed.returncode != 0
    assert result is None
    assert status["code"] == "INVALID_REQUEST"


def test_foreign_factor_instrument_and_bad_provenance_are_rejected(tmp_path):
    for mutate in (
        lambda request: request["factors"].update({"SPY.NYSE": {}}),
        lambda request: request["provenance"].update(
            universe_snapshot_id="sha256:" + "A" * 64
        ),
        lambda request: request["provenance"].update(dataset_version_id="not-a-uuid"),
        lambda request: request["provenance"].update(dataset_manifest_sha256="C" * 64),
        lambda request: request["provenance"].update(extra="not-allowed"),
    ):
        case = tmp_path / str(len(list(tmp_path.iterdir())))
        case.mkdir()
        request = request_for("buy_and_hold")
        mutate(request)
        completed, result, status, _ = run_cli(case, request)
        assert completed.returncode != 0
        assert result is None
        assert status["code"] == "INVALID_REQUEST"


def test_oversized_and_malformed_requests_have_bounded_sanitized_status(tmp_path):
    cases = [b"{" + b"x" * (1024 * 1024), b"{not-json"]
    for index, raw in enumerate(cases):
        case = tmp_path / str(index)
        case.mkdir()
        completed, result, status, _ = run_cli(case, {}, raw=raw)
        assert completed.returncode != 0
        assert result is None
        assert status["code"] in {"REQUEST_TOO_LARGE", "INVALID_JSON"}
        encoded = json.dumps(status).encode("utf-8")
        assert len(encoded) <= 16 * 1024
        assert "xxxxx" not in status["summary"]


def test_stale_result_is_removed_on_failure(tmp_path):
    (tmp_path / "result.json").write_text('{"stale":true}', encoding="utf-8")
    request = request_for("buy_and_hold")
    request["module"] = "os:system"
    _, result, status, _ = run_cli(tmp_path, request)
    assert result is None
    assert status["code"] == "INVALID_REQUEST"
