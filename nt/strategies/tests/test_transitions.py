"""Red-first suite: the registry state machine and the promotion-gate
evidence matrix (FR-STR-003).  Draft -> Validated requires golden+holdout+cost,
Validated -> Paper requires parity + minimum observation window, Paper ->
LiveCandidate requires Phase 3 safety evidence, Retired is Owner-only and
terminal."""

import pytest

from conftest import STRATEGIES, load_package


@pytest.fixture(scope="module")
def reg(registry_module):
    r = registry_module.Registry()
    for sid in STRATEGIES:
        r.register(registry_module.Actor.owner(), load_package(sid).PACKAGE)
    return r


def test_all_five_registered_in_draft(reg, registry_module):
    assert {p["strategy_id"] for p in reg.all_packages()} == set(STRATEGIES)
    for p in reg.all_packages():
        assert p["state"] == registry_module.StrategyState.DRAFT
        assert p["canonical_hash"].startswith("sha256:")


def test_full_promotion_ladder(reg, registry_module):
    golden = registry_module.PromotionEvidence.golden(
        "sha256:golden", "sha256:holdout", "sha256:cost"
    )
    paper = registry_module.PromotionEvidence.paper("parity-1", 30)
    phase3 = registry_module.PromotionEvidence.phase3(
        "phase3-1", registry_module.PHASE3_SAFETY_CHECKS
    )
    record = reg.promote(
        registry_module.Actor.owner(), "buy_and_hold", "1.0.0",
        registry_module.StrategyState.VALIDATED, golden,
    )
    assert (record["from"], record["to"]) == (
        registry_module.StrategyState.DRAFT, registry_module.StrategyState.VALIDATED,
    )
    assert reg.resolve("buy_and_hold", "1.0.0")["state"] == registry_module.StrategyState.VALIDATED

    reg.promote(registry_module.Actor.owner(), "buy_and_hold", "1.0.0",
                registry_module.StrategyState.PAPER, paper)
    assert reg.resolve("buy_and_hold", "1.0.0")["state"] == registry_module.StrategyState.PAPER

    reg.promote(registry_module.Actor.owner(), "buy_and_hold", "1.0.0",
                registry_module.StrategyState.LIVE_CANDIDATE, phase3)
    assert reg.resolve("buy_and_hold", "1.0.0")["state"] == (
        registry_module.StrategyState.LIVE_CANDIDATE
    )

    reg.retire(registry_module.Actor.owner(), "buy_and_hold", "1.0.0")
    assert reg.resolve("buy_and_hold", "1.0.0")["state"] == registry_module.StrategyState.RETIRED


def test_validated_gate_requires_golden_holdout_cost(reg, registry_module):
    with pytest.raises(Exception) as exc:
        reg.promote(
            registry_module.Actor.owner(), "trend_following", "1.0.0",
            registry_module.StrategyState.VALIDATED,
            registry_module.PromotionEvidence.golden("sha256:g", "", "sha256:c"),
        )
    assert exc.value.code == "MISSING_PROMOTION_EVIDENCE"
    assert "holdout" in str(exc.value)
    with pytest.raises(Exception) as exc:
        reg.promote(
            registry_module.Actor.owner(), "trend_following", "1.0.0",
            registry_module.StrategyState.VALIDATED,
            registry_module.PromotionEvidence.paper("parity-1", 30),
        )
    assert exc.value.code == "MISSING_PROMOTION_EVIDENCE"
    assert reg.resolve("trend_following", "1.0.0")["state"] == registry_module.StrategyState.DRAFT


def test_paper_gate_requires_parity_and_window(reg, registry_module):
    golden = registry_module.PromotionEvidence.golden(
        "sha256:g", "sha256:h", "sha256:c"
    )
    reg.promote(registry_module.Actor.owner(), "trend_following", "1.0.0",
                registry_module.StrategyState.VALIDATED, golden)
    with pytest.raises(Exception) as exc:
        reg.promote(
            registry_module.Actor.owner(), "trend_following", "1.0.0",
            registry_module.StrategyState.PAPER,
            registry_module.PromotionEvidence.paper("parity-1", 5),
        )
    assert exc.value.code == "INVALID_PROMOTION"
    with pytest.raises(Exception) as exc:
        reg.promote(
            registry_module.Actor.owner(), "relative_momentum", "1.0.0",
            registry_module.StrategyState.PAPER,
            registry_module.PromotionEvidence.paper("parity-1", 30),
        )
    assert exc.value.code == "INVALID_PROMOTION"
    assert exc.value.message and "Validated" in exc.value.message


def test_live_candidate_gate_requires_phase3_safety(reg, registry_module):
    golden = registry_module.PromotionEvidence.golden(
        "sha256:g", "sha256:h", "sha256:c"
    )
    reg.promote(registry_module.Actor.owner(), "inverse_volatility", "1.0.0",
                registry_module.StrategyState.VALIDATED, golden)
    reg.promote(registry_module.Actor.owner(), "inverse_volatility", "1.0.0",
                registry_module.StrategyState.PAPER,
                registry_module.PromotionEvidence.paper("parity-1", 30))
    missing = set(registry_module.PHASE3_SAFETY_CHECKS) - {"kill_switch"}
    with pytest.raises(Exception) as exc:
        reg.promote(
            registry_module.Actor.owner(), "inverse_volatility", "1.0.0",
            registry_module.StrategyState.LIVE_CANDIDATE,
            registry_module.PromotionEvidence.phase3("bundle", missing),
        )
    assert exc.value.code == "MISSING_PROMOTION_EVIDENCE"
    assert "kill_switch" in str(exc.value)
    # Skipping Paper: a Validated-only strategy promoted straight to
    # LiveCandidate is denied.
    fresh = registry_module.Registry()
    for sid in STRATEGIES:
        fresh.register(registry_module.Actor.owner(), load_package(sid).PACKAGE)
    fresh.promote(
        registry_module.Actor.owner(), "inverse_volatility", "1.0.0",
        registry_module.StrategyState.VALIDATED, golden,
    )
    with pytest.raises(Exception) as exc:
        fresh.promote(
            registry_module.Actor.owner(), "inverse_volatility", "1.0.0",
            registry_module.StrategyState.LIVE_CANDIDATE,
            registry_module.PromotionEvidence.phase3("bundle",
                                                     registry_module.PHASE3_SAFETY_CHECKS),
        )
    assert exc.value.code == "INVALID_PROMOTION"


def test_retired_is_terminal_and_promote_into_draft_denied(reg, registry_module):
    reg.retire(registry_module.Actor.owner(), "dual_momentum", "1.0.0")
    with pytest.raises(Exception) as exc:
        reg.promote(
            registry_module.Actor.owner(), "dual_momentum", "1.0.0",
            registry_module.StrategyState.VALIDATED,
            registry_module.PromotionEvidence.golden("g", "h", "c"),
        )
    assert exc.value.code == "INVALID_PROMOTION"
    assert "terminal" in str(exc.value)


def test_minimum_observation_window_constant_is_21(registry_module):
    assert registry_module.MIN_PAPER_OBSERVATION_SESSIONS == 21
    assert "kill_switch" in registry_module.PHASE3_SAFETY_CHECKS
    assert "fail_closed_restart" in registry_module.PHASE3_SAFETY_CHECKS
    assert "idempotent_order_intent" in registry_module.PHASE3_SAFETY_CHECKS
