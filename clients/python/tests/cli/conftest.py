"""Shared fixtures for the ``efe_tool.cli`` test suite."""

from __future__ import annotations

import io
import os
import sys
from pathlib import Path
from typing import Iterator

import pytest

from efe_tool.cli import logging as cli_logging
from efe_tool.cli.config import MISSING, ResolvedConfig, resolve_config
from efe_tool.cli.parser import build_parser


# ---------------------------------------------------------------------------
# Config fixtures
# ---------------------------------------------------------------------------


@pytest.fixture(autouse=True)
def _clean_env(monkeypatch: pytest.MonkeyPatch) -> None:
    """Scrub every EFE_* / NO_COLOR env var before each test so
    precedence tests are deterministic."""
    for key in list(os.environ.keys()):
        if key.startswith("EFE_") or key == "NO_COLOR":
            monkeypatch.delenv(key, raising=False)


@pytest.fixture(autouse=True)
def _reset_logger() -> Iterator[None]:
    cli_logging.reset_logger_for_test()
    yield
    cli_logging.reset_logger_for_test()


@pytest.fixture
def parser():
    return build_parser()


@pytest.fixture
def parse(parser):
    def _parse(argv: list[str]):
        return parser.parse_args(argv)
    return _parse


@pytest.fixture
def resolve(parse):
    def _resolve(argv: list[str], env: dict[str, str] | None = None) -> ResolvedConfig:
        ns = parse(argv)
        return resolve_config(ns, env=env or {})
    return _resolve


@pytest.fixture
def capture_streams(monkeypatch: pytest.MonkeyPatch):
    """Redirect sys.stdout / sys.stderr to StringIO for capture."""
    out = io.StringIO()
    err = io.StringIO()
    monkeypatch.setattr(sys, "stdout", out)
    monkeypatch.setattr(sys, "stderr", err)
    return out, err


# ---------------------------------------------------------------------------
# Catalog fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def valid_yaml(tmp_path: Path) -> Path:
    content = """\
apiVersion: efe.mode1/v1
kind: AnchorCatalog
metadata:
  name: dummy
  version: 0.0.1
embedder:
  family: sentence-transformers
  model: dummy
  dim: 4
cone_policy:
  max_projection_distance: 0.5
thresholds:
  confidence_high: 0.80
  confidence_medium: 0.50
classes:
  - name: alpha
    description: first
    canonical_sentences:
      - hello
      - hi there
  - name: beta
    description: second
    canonical_sentences:
      - goodbye
      - see you
"""
    p = tmp_path / "dummy.yaml"
    p.write_text(content, encoding="utf-8")
    return p


@pytest.fixture
def invalid_yaml(tmp_path: Path) -> Path:
    content = """\
apiVersion: efe.mode1/v1
kind: AnchorCatalog
metadata:
  name: broken
  version: 0.0.1
embedder:
  family: sentence-transformers
  model: dummy
  dim: 4
classes:
  - name: only_one
    canonical_sentences:
      - hello
thresholds:
  confidence_high: 0.3
  confidence_medium: 0.8
"""
    p = tmp_path / "broken.yaml"
    p.write_text(content, encoding="utf-8")
    return p


@pytest.fixture
def missing_version_yaml(tmp_path: Path) -> Path:
    content = """\
apiVersion: wrong/v0
kind: NotACatalog
classes: []
"""
    p = tmp_path / "missing.yaml"
    p.write_text(content, encoding="utf-8")
    return p
