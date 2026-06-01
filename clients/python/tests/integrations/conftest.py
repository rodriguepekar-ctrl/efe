"""Shared fixtures for the Letter-D integration test suite."""

from __future__ import annotations

import os
import threading
from types import SimpleNamespace
from typing import Any, Iterator

import pytest

from efe_tool.integrations import EfeConfig, EfeFacade
from efe_tool.integrations._facade import request_id_var
from efe_tool.integrations.testing import FakeEfeFacade, make_verdict


# ---------------------------------------------------------------------------
#  Env scrub
# ---------------------------------------------------------------------------


@pytest.fixture(autouse=True)
def _clean_env(monkeypatch: pytest.MonkeyPatch) -> None:
    for k in list(os.environ.keys()):
        if k.startswith("EFE_"):
            monkeypatch.delenv(k, raising=False)


# ---------------------------------------------------------------------------
#  Config factory
# ---------------------------------------------------------------------------


@pytest.fixture
def default_config() -> EfeConfig:
    return EfeConfig(
        endpoint="127.0.0.1:50051",
        catalogs=("support_triage",),
        catalog_search_paths=("catalogs",),
        metrics_enabled=False,
        tracing_enabled=False,
    )


# ---------------------------------------------------------------------------
#  Facade with injected dependencies (no real torch / grpc)
# ---------------------------------------------------------------------------


class _StubEmbedder:
    def __init__(self, name: str):
        self._name = name
    def fingerprint(self) -> str: return f"stub/{self._name}@4"
    def dim(self) -> int: return 4
    def encode(self, text: str): return [0.0, 0.0, 0.0, 1.0]
    def encode_batch(self, texts): return [self.encode(t) for t in texts]


class _StubCatalog:
    def __init__(self, name: str):
        self.name = name
        self.version = "1.0.0"
        self.embedder_fingerprint = "stub/dummy@4"
        self.dim = 4
        self.source_path = None
        self.max_projection_distance = 0.5
        self.thresholds = SimpleNamespace(
            confidence_high=0.80, confidence_medium=0.50,
        )


class _StubClassifyResult:
    def __init__(self, label: str = "alpha", confidence: float = 0.85,
                 reject: bool = False, rounds: int = 4):
        self.label = label
        self.confidence = confidence
        self.reject = reject
        self.rounds = rounds
        self.alignment_delta = 0.1
        self.extra = {"zone": "trusted" if confidence >= 0.80 else "warn"}


class _StubEfeClient:
    """A tiny stub of EfeClient that records every classify call."""

    def __init__(self, *, result: Any | None = None, raise_exc: Exception | None = None):
        self._result = result or _StubClassifyResult()
        self._raise = raise_exc
        self.calls: list[tuple[str, str]] = []
        self.closed = False

    def classify(self, text: str, catalog: Any, **kw: Any) -> Any:
        self.calls.append((text, catalog.name))
        if self._raise is not None:
            raise self._raise
        return self._result

    def healthcheck(self) -> bool:
        return not self.closed

    def close(self) -> None:
        self.closed = True


@pytest.fixture
def stub_embedder() -> _StubEmbedder:
    return _StubEmbedder("dummy")


@pytest.fixture
def stub_client() -> _StubEfeClient:
    return _StubEfeClient()


@pytest.fixture
def stub_catalog() -> _StubCatalog:
    return _StubCatalog("support_triage")


@pytest.fixture
def facade_with_stubs(default_config, stub_embedder,
                      stub_client, stub_catalog) -> Iterator[EfeFacade]:
    """An EfeFacade with its heavy dependencies swapped for stubs.

    Bypasses preload() to avoid touching torch / grpcio.
    """
    f = EfeFacade(default_config)
    f._embedder = stub_embedder
    f._client = stub_client
    f._catalogs = {stub_catalog.name: stub_catalog}
    f._is_ready = True
    f._stats.is_ready = True
    f._stats.catalogs_loaded = (stub_catalog.name,)
    try:
        yield f
    finally:
        f._is_closed = True  # no real close needed


# ---------------------------------------------------------------------------
#  Request id context var reset
# ---------------------------------------------------------------------------


@pytest.fixture(autouse=True)
def _reset_request_id() -> Iterator[None]:
    token = request_id_var.set(None)
    try:
        yield
    finally:
        request_id_var.reset(token)
