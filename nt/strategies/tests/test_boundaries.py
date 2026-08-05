"""Red-first suite: authorization and scope boundaries — unauthorized
promotion is denied + audited, arbitrary Member code upload is denied,
Member changes are schema-bound configs only, unsupported market/cadence is a
typed error."""

import pytest

from conftest import STRATEGIES, load_package


@pytest.fixture(scope="module")
def reg(registry_module):
    r = registry_module.Registry()
    for sid in STRATEGIES:
        r.register(registry_module.Actor.owner(), load_package(sid).PACKAGE)
    return r, registry_module


def test_member_registration_denied_and_audited(reg):
    registry, rm = reg
    with pytest.raises(Exception) as exc:
        registry.register(rm.Actor.member("alice"), load_package("buy_and_hold").PACKAGE)
    assert exc.value.code == "UNAUTHORIZED"
    denied = [e for e in registry.audit()
              if e["action"] == "REGISTER" and e["outcome"] == "DENIED"]
    assert denied and denied[-1]["actor"] == "member:alice"


def test_member_promotion_denied_and_audited(reg):
    registry, rm = reg
    with pytest.raises(Exception) as exc:
        registry.promote(
            rm.Actor.member("alice"), "dual_momentum", "1.0.0",
            rm.StrategyState.VALIDATED,
            rm.PromotionEvidence.golden("g", "h", "c"),
        )
    assert exc.value.code == "UNAUTHORIZED"
    denied = [e for e in registry.audit()
              if e["action"] == "PROMOTE" and e["outcome"] == "DENIED"]
    entry = denied[-1]
    assert entry["actor"] == "member:alice"
    assert entry["strategy_id"] == "dual_momentum"
    assert entry["to_state"] == rm.StrategyState.VALIDATED
    assert "Owner" in entry["reason"]
    assert registry.resolve("dual_momentum", "1.0.0")["state"] == rm.StrategyState.DRAFT


def test_member_code_upload_denied(reg):
    registry, rm = reg
    with pytest.raises(Exception) as exc:
        registry.deploy_code(
            rm.Actor.member("alice"),
            "def evil(): import os; os.system('rm -rf /')",
        )
    assert exc.value.code == "MEMBER_CODE_DENIED"
    denied = [e for e in registry.audit()
              if e["action"] == "DEPLOY_CODE" and e["outcome"] == "DENIED"]
    assert denied and denied[-1]["actor"] == "member:alice"
    registry.deploy_code(rm.Actor.owner(), "def run(ctx): return ctx.targets")
    assert len(registry.deployments()) == 1


def test_member_config_is_schema_bound(reg):
    registry, rm = reg
    config = registry.apply_member_config(
        rm.Actor.member("alice"),
        "buy_and_hold",
        "1.0.0",
        {"benchmark_instrument": "069500.KRX", "target_weight": 0.8,
         "rebalance_cadence": "monthly"},
    )
    assert config["strategy_version"] == "1.0.0"
    assert config["parameters"]["target_weight"] == 0.8
    assert len(registry.configs()) == 1

    with pytest.raises(Exception) as exc:
        registry.apply_member_config(
            rm.Actor.member("alice"), "buy_and_hold", "1.0.0",
            {"benchmark_instrument": "069500.KRX", "target_weight": 1.5},
        )
    assert exc.value.code == "INVALID_PARAMETERS"
    with pytest.raises(Exception) as exc:
        registry.apply_member_config(
            rm.Actor.member("alice"), "buy_and_hold", "1.0.0",
            {"benchmark_instrument": "069500.KRX", "target_weight": 0.5, "leverage": 3},
        )
    assert exc.value.code == "INVALID_PARAMETERS"

    package = registry.resolve("buy_and_hold", "1.0.0")
    assert package["state"] == rm.StrategyState.DRAFT
    assert package["default_parameters"]["target_weight"] == 1.0


def test_unsupported_market_and_cadence_typed_errors(reg):
    registry, rm = reg
    with pytest.raises(Exception) as exc:
        rm.parse_market("us")
    assert exc.value.code == "UNSUPPORTED_MARKET"
    with pytest.raises(Exception) as exc:
        rm.parse_cadence("intraday")
    assert exc.value.code == "UNSUPPORTED_CADENCE"
    with pytest.raises(Exception) as exc:
        rm.parse_asset_class("equity")
    assert exc.value.code == "UNSUPPORTED_ASSET_CLASS"

    no_market = dict(load_package("buy_and_hold").PACKAGE)
    no_market["strategy_id"] = "no_market_strategy"
    no_market["markets"] = []
    with pytest.raises(Exception) as exc:
        registry.register(rm.Actor.owner(), no_market)
    assert exc.value.code == "INVALID_PACKAGE"

    no_cadence = dict(load_package("buy_and_hold").PACKAGE)
    no_cadence["strategy_id"] = "no_cadence_strategy"
    no_cadence["cadences"] = []
    with pytest.raises(Exception) as exc:
        registry.register(rm.Actor.owner(), no_cadence)
    assert exc.value.code == "INVALID_PACKAGE"


def test_invalid_package_definitions_rejected(reg):
    registry, rm = reg
    bad_defaults = dict(load_package("buy_and_hold").PACKAGE)
    bad_defaults["strategy_id"] = "bad_defaults"
    bad_defaults["default_parameters"] = {
        "benchmark_instrument": "069500.KRX", "target_weight": 2.0,
    }
    with pytest.raises(Exception) as exc:
        registry.register(rm.Actor.owner(), bad_defaults)
    assert exc.value.code == "INVALID_PACKAGE"

    pre_validated = dict(load_package("buy_and_hold").PACKAGE)
    pre_validated["strategy_id"] = "pre_validated"
    pre_validated["state"] = rm.StrategyState.VALIDATED
    with pytest.raises(Exception) as exc:
        registry.register(rm.Actor.owner(), pre_validated)
    assert exc.value.code == "INVALID_PACKAGE"

    inconsistent = dict(load_package("buy_and_hold").PACKAGE)
    inconsistent["strategy_id"] = "inconsistent"
    inconsistent["required_factors"] = []
    inconsistent["minimum_lookback_sessions"] = 100
    with pytest.raises(Exception) as exc:
        registry.register(rm.Actor.owner(), inconsistent)
    assert exc.value.code == "INVALID_PACKAGE"


def test_audit_is_append_only_and_ordered(reg):
    registry, rm = reg
    audit = registry.audit()
    # Every baseline registration is audited (5 approvals) plus every denial
    # exercised by the tests above.
    approved_registers = [
        e for e in audit if e["action"] == "REGISTER" and e["outcome"] == "APPROVED"
    ]
    assert len(approved_registers) == len(STRATEGIES), "every registration is audited"
    assert len(audit) >= len(STRATEGIES)
    seqs = [e["seq"] for e in audit]
    assert seqs == sorted(seqs) and len(set(seqs)) == len(seqs)
    assert any(e["outcome"] == "APPROVED" for e in audit)
    assert any(e["outcome"] == "DENIED" for e in audit)
