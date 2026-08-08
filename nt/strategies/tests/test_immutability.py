"""Red-first suite: immutability of published versions and typed version
resolution (FR-STR-001).  After a new release, old runs still resolve the
ORIGINAL immutable version."""

import pytest

from strategy_helpers import load_package


@pytest.fixture(scope="module")
def reg(registry_module):
    r = registry_module.Registry()
    r.register(registry_module.Actor.owner(), load_package("buy_and_hold").PACKAGE)
    return r, registry_module


def test_mutated_published_version_rejected(reg):
    registry, rm = reg
    original = registry.resolve("buy_and_hold", "1.0.0")
    mutated = dict(original)
    mutated["risk_description"] = "mutated risk"
    with pytest.raises(Exception) as exc:
        registry.register(rm.Actor.owner(), mutated)
    assert exc.value.code == "IMMUTABLE_VERSION"
    resolved = registry.resolve("buy_and_hold", "1.0.0")
    assert resolved["canonical_hash"] == original["canonical_hash"]
    assert resolved["risk_description"] == original["risk_description"]


def test_old_runs_resolve_original_version_after_new_release(reg):
    registry, rm = reg
    original_hash = registry.resolve("buy_and_hold", "1.0.0")["canonical_hash"]
    original_risk = registry.resolve("buy_and_hold", "1.0.0")["risk_description"]

    next_version = dict(registry.resolve("buy_and_hold", "1.0.0"))
    next_version["version"] = "1.1.0"
    next_version["risk_description"] = "v1.1 risk model update"
    registry.register(rm.Actor.owner(), next_version)

    assert registry.resolve_latest("buy_and_hold")["version"] == "1.1.0"
    old = registry.resolve("buy_and_hold", "1.0.0")
    assert old["version"] == "1.0.0"
    assert old["canonical_hash"] == original_hash
    assert old["risk_description"] == original_risk


def test_version_resolution_is_typed(reg):
    registry, rm = reg
    with pytest.raises(Exception) as exc:
        registry.resolve("no_such_strategy", "1.0.0")
    assert exc.value.code == "UNKNOWN_STRATEGY"
    with pytest.raises(Exception) as exc:
        registry.resolve("buy_and_hold", "9.9.9")
    assert exc.value.code == "UNKNOWN_VERSION"
