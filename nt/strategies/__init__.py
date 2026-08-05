"""The five versioned baseline strategy packages (plan Todo 17, FR-STR-004).

Each package under this directory carries immutable metadata (id + SemVer +
JSON Schema + defaults + supported market/cadence + required factors/lookback
+ risk), an engine-independent target generator, an NT execution adapter, and
golden fixtures.  The registry (``strategies._registry``) mirrors the Rust
registry in ``crates/selector`` and enforces the promotion gates.
"""

from strategies._registry import Actor, Registry, baseline_packages


def build_registry() -> Registry:
    """A registry with the five baseline packages registered in Draft."""
    registry = Registry()
    for package in baseline_packages():
        registry.register(Actor.owner(), package)
    return registry


__all__ = ["Registry", "Actor", "baseline_packages", "build_registry"]
