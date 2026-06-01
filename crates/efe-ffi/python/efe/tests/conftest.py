"""Shared pytest fixtures for the ``efe`` test suite."""

from __future__ import annotations

import json
import tempfile
from pathlib import Path
from typing import Iterator

import pytest

# Skip the entire module when the wheel is not yet built (e.g. on
# a fresh clone before running ``maturin develop``).
pytest.importorskip("efe._native")

import efe  # noqa: E402


@pytest.fixture(scope="session")
def bundle_path(tmp_path_factory: pytest.TempPathFactory) -> Path:
    """A directory representing a 'bundle' (D25 stub : just a path)."""
    p = tmp_path_factory.mktemp("efe-bundle")
    return p


@pytest.fixture(scope="session")
def bundle(bundle_path: Path) -> efe.Bundle:
    """Loaded bundle wrapper."""
    return efe.load_bundle(str(bundle_path))


@pytest.fixture(scope="session")
def catalog_path(tmp_path_factory: pytest.TempPathFactory) -> Path:
    """A catalog JSON file with 5 fixed labels."""
    p = tmp_path_factory.mktemp("efe-cat") / "router-v1.json"
    payload = {
        "id": "router-v1",
        "version": "1.0.0",
        "labels": [
            "billing-dispute",
            "tech-support",
            "refund-request",
            "general-inquiry",
            "other",
        ],
    }
    p.write_text(json.dumps(payload))
    return p


@pytest.fixture
def client(bundle: efe.Bundle, catalog_path: Path) -> Iterator[efe.EfeNativeClient]:
    """A fresh EfeNativeClient for each test."""
    c = efe.EfeNativeClient(bundle, str(catalog_path), seed=42)
    yield c
    c.close()


@pytest.fixture
def fixture_texts() -> list[str]:
    """A small set of representative input texts."""
    return [
        "I want to dispute the charge",
        "My internet is not working",
        "Please refund my order #1234",
        "How do I update my address",
        "Hello",
    ]
