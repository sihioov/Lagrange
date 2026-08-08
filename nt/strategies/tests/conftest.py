"""Shared fixtures for the nt/strategies test suite (plan Todo 17).

Fixtures only. The plain helpers live in `strategy_helpers.py`, because
pytest imports every `conftest.py` under `nt/` by the same basename and a
test writing ``from conftest import ...`` gets whichever one was collected
first — see that module's docstring for the failure it caused.
"""

import importlib

import pytest

# Imported for the `sys.path` setup it performs: `strategies` and the
# hyphenated `custom-data` package both live under `nt/`, which has to be on
# the path before any fixture below can import either.
from strategy_helpers import NT_ROOT  # noqa: F401


@pytest.fixture(scope="session")
def events():
    """The custom-data.session_events module (Todo 13 custom events)."""
    return importlib.import_module("custom-data.session_events")


@pytest.fixture(scope="session")
def registry_module():
    """The strategies._registry module."""
    return importlib.import_module("strategies._registry")


@pytest.fixture(scope="session")
def execution_module():
    """The strategies._execution module (adapter base)."""
    return importlib.import_module("strategies._execution")
