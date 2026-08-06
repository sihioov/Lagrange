"""Shared helpers for nt/backtest-worker tests (Todo 20).

Deliberately NOT a conftest.py: a conftest under this tree would join the
known `import conftest` shadow class (tests/golden/conftest.py vs
nt/custom-data/tests/conftest.py) when the full `uv run pytest -q` collects.
Tests import this module explicitly.
"""
from __future__ import annotations

import os
import sys
from pathlib import Path

WORKER_ROOT = Path(__file__).resolve().parent.parent   # nt/backtest-worker
NT_ROOT = WORKER_ROOT.parent                            # nt/

if str(WORKER_ROOT) not in sys.path:
    sys.path.insert(0, str(WORKER_ROOT))

IS_WINDOWS = os.name == "nt"


def child_env(**extra: str) -> dict[str, str]:
    env = os.environ.copy()
    env["PYTHONPATH"] = os.pathsep.join(
        [str(NT_ROOT), str(NT_ROOT / "strategies"), str(WORKER_ROOT)]
    )
    env.update(extra)
    return env
