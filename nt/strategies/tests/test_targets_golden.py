"""Red-first suite: every target generator reproduces its golden fixtures
(FR-STR-004: each strategy has example settings and a baseline expectation).
Generators produce TargetPortfolio-shaped results (Todo 16 boundary: targets,
never orders)."""

import pytest

from strategy_helpers import STRATEGIES, load_golden, load_target


@pytest.mark.parametrize("sid", STRATEGIES)
def test_golden_cases_produce_expected_targets(sid):
    golden = load_golden(sid)
    gen = load_target(sid)
    for case in golden["cases"]:
        portfolio = gen.generate_target(
            case["params"],
            case.get("factors", {}),
            case["as_of"],
            case.get("universe"),
        )
        assert portfolio["strategy_version"] == f"{sid}@{golden['version']}"
        expected = case["expected"]
        got = [
            (t["instrument_id"], round(t["target_weight"], 4))
            for t in portfolio["targets"]
        ]
        assert got == [
            (e["instrument_id"], e["target_weight"]) for e in expected["targets"]
        ], f"{sid} case {case['name']}: target mismatch"
        assert portfolio["cash_weight"] == pytest.approx(
            expected["cash_weight"], abs=1e-9
        ), f"{sid} case {case['name']}: cash mismatch"
        assert portfolio["portfolio_snapshot_id"].startswith("sha256:")


@pytest.mark.parametrize("sid", STRATEGIES)
def test_golden_targets_are_explainable(sid):
    golden = load_golden(sid)
    gen = load_target(sid)
    for case in golden["cases"]:
        portfolio = gen.generate_target(
            case["params"],
            case.get("factors", {}),
            case["as_of"],
            case.get("universe"),
        )
        for target in portfolio["targets"]:
            assert target["rank"] >= 1
            assert target["reasons"], "every target carries structured reasons"
            assert target["factors"] is not None
        assert isinstance(portfolio["exclusions"], list)
        assert isinstance(portfolio["portfolio_reasons"], list)


@pytest.mark.parametrize("sid", STRATEGIES)
def test_generation_is_deterministic(sid):
    golden = load_golden(sid)
    gen = load_target(sid)
    case = golden["cases"][0]
    first = gen.generate_target(
        case["params"], case.get("factors", {}), case["as_of"], case.get("universe")
    )
    second = gen.generate_target(
        case["params"], case.get("factors", {}), case["as_of"], case.get("universe")
    )
    assert first == second


def test_missing_required_factor_is_typed_error():
    gen = load_target("relative_momentum")
    with pytest.raises(Exception) as exc:
        gen.generate_target(
            {"top_n": 3, "lookback_months": 12},
            {"069500.KRX": {"return_12m": 0.1}},
            "2020-02-03",
            ["069500.KRX"],
        )
    assert getattr(exc.value, "code", None) == "MISSING_REQUIRED_FACTOR"


def test_invalid_parameters_are_typed_error():
    gen = load_target("buy_and_hold")
    with pytest.raises(Exception) as exc:
        gen.generate_target(
            {"benchmark_instrument": "069500.KRX", "target_weight": 2.0},
            {},
            "2020-02-03",
            ["069500.KRX"],
        )
    assert getattr(exc.value, "code", None) == "INVALID_PARAMETERS"


def test_null_mandatory_factor_excludes_with_reason():
    gen = load_target("inverse_volatility")
    portfolio = gen.generate_target(
        {"vol_window": 60, "max_weight": 0.3},
        {
            "069500.KRX": {"vol_60": 0.15},
            "102110.KRX": {"vol_60": None},
        },
        "2020-02-03",
        ["069500.KRX", "102110.KRX"],
    )
    excluded = {e["instrument_id"] for e in portfolio["exclusions"]}
    assert "102110.KRX" in excluded
    codes = [r["code"] for e in portfolio["exclusions"] for r in e["reasons"]]
    assert "EXCLUDED_MANDATORY_FACTOR_NULL" in codes
