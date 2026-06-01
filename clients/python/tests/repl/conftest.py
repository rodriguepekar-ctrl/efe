"""Shared fixtures for the ``efe_tool.repl`` test suite."""

from __future__ import annotations

import os
from pathlib import Path
from types import SimpleNamespace
from typing import Any, Iterator

import pytest

from efe_tool.cli import logging as cli_logging
from efe_tool.cli.config import ResolvedConfig
from efe_tool.cli.logging import Logger
from efe_tool.repl.session import SessionState


# ---------------------------------------------------------------------------
#  Env scrub
# ---------------------------------------------------------------------------


@pytest.fixture(autouse=True)
def _clean_env(monkeypatch: pytest.MonkeyPatch) -> None:
    for key in list(os.environ.keys()):
        if key.startswith("EFE_") or key in {"NO_COLOR", "XDG_DATA_HOME"}:
            monkeypatch.delenv(key, raising=False)


@pytest.fixture(autouse=True)
def _reset_logger() -> Iterator[None]:
    cli_logging.reset_logger_for_test()
    yield
    cli_logging.reset_logger_for_test()


# ---------------------------------------------------------------------------
#  ResolvedConfig factory
# ---------------------------------------------------------------------------


def _default_cfg(**overrides: Any) -> ResolvedConfig:
    base = dict(
        command="repl",
        endpoint="127.0.0.1:50051",
        tls=False,
        tls_ca=None,
        jwt=None,
        tenant=None,
        catalog_path=None,
        embedder_model="dummy-model",
        budget_ms=5000,
        timeout_s=10.0,
        max_rounds=16,
        strict=False,
        format="human",
        pretty=False,
        with_diagnostics=False,
        log_level="info",
        log_format="text",
        quiet=False,
        no_color=True,            # tests default: no colors, deterministic
        request_id="rid-test",
        progress="auto",
    )
    base.update(overrides)
    return ResolvedConfig(**base)


@pytest.fixture
def cfg() -> ResolvedConfig:
    return _default_cfg()


# ---------------------------------------------------------------------------
#  Catalog fixtures (valid YAML + compiled.json for embedder-free loading)
# ---------------------------------------------------------------------------


@pytest.fixture
def yaml_catalog(tmp_path: Path) -> Path:
    """A minimal valid catalog YAML with two classes."""
    p = tmp_path / "dummy.yaml"
    p.write_text(
        "apiVersion: efe.mode1/v1\n"
        "kind: AnchorCatalog\n"
        "metadata:\n  name: dummy\n  version: 0.0.1\n"
        "embedder:\n  family: sentence-transformers\n  model: dummy\n  dim: 4\n"
        "cone_policy:\n  max_projection_distance: 0.5\n"
        "thresholds:\n  confidence_high: 0.80\n  confidence_medium: 0.50\n"
        "classes:\n"
        "  - name: alpha\n    description: first class\n"
        "    color: blue\n"
        "    canonical_sentences: [hello, hi]\n"
        "  - name: beta\n    description: second class\n"
        "    color: green\n"
        "    canonical_sentences: [bye, goodbye]\n",
        encoding="utf-8",
    )
    return p


@pytest.fixture
def compiled_catalog(yaml_catalog: Path) -> Path:
    """Writes a compatible ``.compiled.json`` next to ``yaml_catalog`` so
    the catalog loader can be used without a real embedder."""
    import hashlib
    import json

    # The hash algorithm below mirrors :meth:`AnchorCatalog._compiled_hash`.
    declared_fp = "sentence-transformers/dummy@4"
    version = "0.0.1"
    classes_raw = [
        ("alpha", ("hello", "hi")),
        ("beta", ("bye", "goodbye")),
    ]

    h = hashlib.sha256()
    h.update(version.encode())
    h.update(b"\0")
    h.update(declared_fp.encode())
    h.update(b"\0")
    for name, sents in classes_raw:
        h.update(name.encode())
        h.update(b"\0")
        for s in sents:
            h.update(s.encode())
            h.update(b"\0")

    # Tiny L2-normalised anchors (dim 4).
    anchors = [
        [1.0, 0.0, 0.0, 0.0],  # alpha
        [0.0, 1.0, 0.0, 0.0],  # beta
    ]
    payload = {
        "hash": h.hexdigest(),
        "version": version,
        "embedder_fingerprint": declared_fp,
        "classes": [c for c, _ in classes_raw],
        "anchors": anchors,
    }
    out = yaml_catalog.with_name(f"{yaml_catalog.stem}.compiled.json")
    out.write_text(json.dumps(payload), encoding="utf-8")
    return out


# ---------------------------------------------------------------------------
#  SessionState factory
# ---------------------------------------------------------------------------


@pytest.fixture
def state(cfg: ResolvedConfig) -> SessionState:
    """A ``SessionState`` with no real catalog / client / embedder."""
    return SessionState.from_boot(cfg)


# ---------------------------------------------------------------------------
#  Fake client / fake catalog for unit tests
# ---------------------------------------------------------------------------


class FakeClient:
    """A client stub that returns a canonical ``ClassifyResult``-like
    object without touching the network."""

    def __init__(self, *, label: str = "alpha", confidence: float = 0.7,
                 rounds: int = 4, reject: bool = False) -> None:
        self.label = label
        self.confidence = confidence
        self.rounds = rounds
        self.reject = reject
        self.calls: list[tuple[str, dict]] = []

    def healthcheck(self) -> bool:
        return True

    def classify(self, text: str, catalog: Any, **kw: Any) -> Any:
        self.calls.append((text, kw))
        return SimpleNamespace(
            label=self.label,
            confidence=self.confidence,
            alignment_delta=0.1,
            rounds=self.rounds,
            reject=self.reject,
            total_ms=50,
            text_hash="hash",
            swarm_id="swarm-1",
            top_cosine=0.9,
            sub_scores=None,
            extra={"zone": "trusted"},
        )

    def close(self) -> None:
        pass


@pytest.fixture
def fake_client() -> FakeClient:
    return FakeClient()


class FakeCatalog:
    """Minimal duck-typed catalog for rendering / state tests."""

    def __init__(self, name: str = "fake", version: str = "1.0.0"):
        self.name = name
        self.version = version
        self.embedder_fingerprint = "sentence-transformers/dummy@4"
        self.dim = 4
        self.source_path = None
        self.classes = (
            SimpleNamespace(
                name="alpha", description="",
                canonical_sentences=("hello",), color="blue",
            ),
            SimpleNamespace(
                name="beta", description="",
                canonical_sentences=("bye",), color="green",
            ),
        )
        self.thresholds = SimpleNamespace(
            confidence_high=0.80, confidence_medium=0.50,
        )
        self.max_projection_distance = 0.5


@pytest.fixture
def fake_catalog() -> FakeCatalog:
    return FakeCatalog()


@pytest.fixture
def state_with_fakes(state: SessionState, fake_client: FakeClient,
                     fake_catalog: FakeCatalog) -> SessionState:
    state.client = fake_client
    state.catalog = fake_catalog
    return state


# ---------------------------------------------------------------------------
#  Logger stub
# ---------------------------------------------------------------------------


class NoopLogger:
    """A Logger-compatible no-op, used in unit tests to silence stderr."""
    def debug(self, *a, **kw): pass
    def info(self, *a, **kw): pass
    def warn(self, *a, **kw): pass
    def error(self, *a, **kw): pass


@pytest.fixture
def noop_log() -> NoopLogger:
    return NoopLogger()
