"""Bootstrap smoke test for the pinned NautilusTrader environment (Todo 1).

Proves the approved nautilus_trader==1.231.0 pin actually resolves and imports in
the uv-managed CPython 3.12 environment. Real nt tests land with their todos
(strategies: 17, custom-data: 13, backtest-worker: 20, paper-runner: 30,
live-node: 37-42). ADR-0001: no Python polars pin; NT provides pandas/pyarrow.
"""


def test_import_nautilus() -> None:
    import nautilus_trader as nt

    assert nt.__version__ == "1.231.0"
