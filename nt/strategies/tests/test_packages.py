"""Red-first suite: the five baseline packages validate their metadata
(ID / SemVer / JSON Schema / defaults / market / cadence / required factors /
lookback / risk) and cross-match the Rust registry baseline."""

from pathlib import Path

import pytest
from jsonschema import Draft202012Validator, validate as js_validate

from strategy_helpers import NT_ROOT, STRATEGIES, load_golden, load_package

#: Mirror of the Rust baseline metadata (selector::baseline).  The Python
#: packages must agree with the Rust registry on every cross-visible field.
EXPECTED = {
    "buy_and_hold": {"required_factors": [], "lookback": 0},
    "trend_following": {"required_factors": ["trend_50", "trend_200"], "lookback": 200},
    "relative_momentum": {"required_factors": ["momentum_12_1"], "lookback": 252},
    "dual_momentum": {"required_factors": ["return_12m"], "lookback": 252},
    "inverse_volatility": {"required_factors": ["vol_60"], "lookback": 60},
}


def test_five_baseline_packages_present():
    assert set(STRATEGIES) == {
        "buy_and_hold",
        "trend_following",
        "relative_momentum",
        "dual_momentum",
        "inverse_volatility",
    }


@pytest.mark.parametrize("sid", STRATEGIES)
def test_package_metadata_validates(sid):
    mod = load_package(sid)
    p = mod.PACKAGE
    assert p["strategy_id"] == sid
    # SemVer
    parts = p["version"].split(".")
    assert len(parts) == 3 and all(part.isdigit() for part in parts)
    # JSON Schema: valid draft-2020-12 object schema with properties
    Draft202012Validator.check_schema(p["parameter_schema"])
    assert p["parameter_schema"]["type"] == "object"
    assert "properties" in p["parameter_schema"]
    # Defaults validate against the package's own schema
    js_validate(p["default_parameters"], p["parameter_schema"])
    # Supported market / asset class / cadence (MVP: KRX ETF daily only)
    assert p["markets"] == ["krx"]
    assert p["asset_classes"] == ["etf"]
    assert p["cadences"] == ["daily"]
    # Risk description and description are present
    assert p["risk_description"].strip()
    assert p["description"].strip()
    # Required factors + lookback
    assert isinstance(p["required_factors"], list)
    assert p["minimum_lookback_sessions"] >= 0
    if not p["required_factors"]:
        assert p["minimum_lookback_sessions"] == 0
    # References
    assert p["target_generator_ref"].endswith(":generate_target")
    assert p["nt_adapter_ref"].startswith(f"nt.strategies.{sid}.adapter:")
    # Golden fixture exists
    fixture = Path(p["golden_fixture_refs"][0])
    assert (NT_ROOT.parent / fixture).is_file(), f"golden fixture missing: {fixture}"


@pytest.mark.parametrize("sid", STRATEGIES)
def test_package_matches_rust_baseline(sid):
    p = load_package(sid).PACKAGE
    expected = EXPECTED[sid]
    assert set(p["required_factors"]) == set(expected["required_factors"])
    assert p["minimum_lookback_sessions"] == expected["lookback"]
    assert p["version"] == "1.0.0"


def test_default_parameters_are_documented():
    assert load_package("buy_and_hold").DEFAULT_PARAMETERS["benchmark_instrument"] == "069500.KRX"
    assert load_package("buy_and_hold").DEFAULT_PARAMETERS["target_weight"] == 1.0
    assert load_package("trend_following").DEFAULT_PARAMETERS == {
        "benchmark_instrument": "069500.KRX",
        "fast_ma": 50,
        "slow_ma": 200,
    }
    assert load_package("relative_momentum").DEFAULT_PARAMETERS == {"top_n": 3, "lookback_months": 12}
    assert load_package("dual_momentum").DEFAULT_PARAMETERS == {
        "absolute_threshold": 0.0,
        "lookback_months": 12,
    }
    assert load_package("inverse_volatility").DEFAULT_PARAMETERS == {
        "vol_window": 60,
        "max_weight": 0.3,
    }


@pytest.mark.parametrize("sid", STRATEGIES)
def test_golden_fixture_matches_package_identity(sid):
    golden = load_golden(sid)
    p = load_package(sid).PACKAGE
    assert golden["strategy_id"] == p["strategy_id"]
    assert golden["version"] == p["version"]
    assert golden["cases"], "every package has at least one golden case"
