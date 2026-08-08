"""Shared fixtures for the nt/custom-data test suite (Todo 13).

Fixtures only. The plain helpers live in `curated_helpers.py`, because pytest
imports every `conftest.py` under `nt/` by the same basename and a test
writing ``from conftest import ...`` gets whichever directory was collected
first — see that module's docstring for the failure this caused.
"""
from __future__ import annotations

import pytest

from curated_helpers import (
    load_builder_module,
    load_events_module,
    write_curated_fixture,
)


@pytest.fixture(scope="session")
def events():
    """The custom-data.session_events module (classes + validation)."""
    return load_events_module()


@pytest.fixture(scope="session")
def builder():
    """The custom-data.catalog_builder module."""
    return load_builder_module()


@pytest.fixture(scope="session")
def curated_root(tmp_path_factory):
    """A synthetic curated zone with the golden 3-instrument fixture."""
    root = tmp_path_factory.mktemp("curated-root")
    write_curated_fixture(root)
    return root
