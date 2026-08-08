"""Test path setup for the live-node suite (plan Todo 41).

`nt/live-node` is a hyphenated directory, so it cannot be a Python package
name and is not installed (`[tool.uv] package = false`). The package inside it
is `live_node`, and importing it needs its PARENT — the `live-node` directory
— on `sys.path`.

A conftest rather than a `helpers.py` import: pytest loads conftest before it
imports any test module in the directory, so every test file gets the path
without having to remember to import something first. `nt/backtest-worker`
uses the helpers form and `nt/custom-data` uses conftest; conftest is the one
that cannot be forgotten.
"""
from __future__ import annotations

import sys
from pathlib import Path

LIVE_NODE_ROOT = Path(__file__).resolve().parents[1]  # nt/live-node

if str(LIVE_NODE_ROOT) not in sys.path:
    sys.path.insert(0, str(LIVE_NODE_ROOT))
