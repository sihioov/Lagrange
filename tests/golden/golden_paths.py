"""Paths the golden-manifest harness needs (Todo 6).

Deliberately NOT in `conftest.py`. pytest puts each test directory on
``sys.path`` and imports conftest by BASENAME, so every `conftest.py` in the
repository competes for the single module name `conftest`; a test writing
``from conftest import ...`` gets whichever directory was collected first.

`nt/custom-data/tests` and `nt/strategies/tests` hit exactly that — each
suite passed alone while `pytest nt` failed at import — and this directory is
the third member of the same set. Keeping the constant here means a run that
collects this suite alongside them resolves it by a name nothing else claims.
"""
from __future__ import annotations

import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPTS_GOLDEN = REPO_ROOT / "scripts" / "golden"
if str(SCRIPTS_GOLDEN) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_GOLDEN))

GOLDEN_PY = SCRIPTS_GOLDEN / "golden.py"
