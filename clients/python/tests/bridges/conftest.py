"""Shared fixtures for the bridges test suite (Letter C)."""

from __future__ import annotations

import os
from types import SimpleNamespace
from typing import Any

import pytest

from efe_tool.bridges.mock import (
    MockAnthropic,
    MockResponse,
    make_usage,
    text_block,
    tool_use_block,
)


@pytest.fixture(autouse=True)
def _clean_env(monkeypatch: pytest.MonkeyPatch) -> None:
    """Scrub EFE_* / ANTHROPIC_* env vars before each test."""
    for k in list(os.environ.keys()):
        if k.startswith(("EFE_", "ANTHROPIC_")) or k == "NO_COLOR":
            monkeypatch.delenv(k, raising=False)


class FakeClassifyResult:
    """Duck-type of :class:`efe_tool.types.ClassifyResult`."""

    def __init__(
        self, label: str = "alpha", confidence: float = 0.85,
        rounds: int = 4, reject: bool = False, alignment_delta: float = 0.1,
    ) -> None:
        self.label = label
        self.confidence = confidence
        self.rounds = rounds
        self.reject = reject
        self.alignment_delta = alignment_delta

    def as_tool_result(self) -> dict[str, Any]:
        return {
            "label": self.label,
            "confidence": self.confidence,
            "alignment_delta": self.alignment_delta,
            "rounds": self.rounds,
            "reject": self.reject,
        }


class FakeEfeClient:
    """Drop-in for :class:`EfeClient` — no gRPC, records calls."""

    def __init__(self, *, result: Any = None, raise_exc: Exception | None = None):
        self._result = result or FakeClassifyResult()
        self._raise = raise_exc
        self.calls: list[tuple] = []

    def classify(self, text: str, catalog: Any, **kw: Any) -> Any:
        self.calls.append((text, getattr(catalog, "name", "?"), kw))
        if self._raise is not None:
            raise self._raise
        return self._result

    def close(self) -> None:
        pass


class FakeCatalog:
    def __init__(self, name: str = "support_triage"):
        self.name = name
        self.version = "1.0.0"
        self.embedder_fingerprint = "sentence-transformers/dummy@4"
        self.dim = 4


@pytest.fixture
def fake_catalog() -> FakeCatalog:
    return FakeCatalog()


@pytest.fixture
def fake_efe_client() -> FakeEfeClient:
    return FakeEfeClient()


@pytest.fixture
def catalogs(fake_catalog) -> dict:
    return {fake_catalog.name: fake_catalog}


@pytest.fixture
def mock_tool_then_ack() -> MockAnthropic:
    return MockAnthropic(responses=[
        MockResponse(
            stop_reason="tool_use",
            content=[tool_use_block(
                "toolu_1", "efe_classify",
                {"text": "hi", "catalog": "support_triage"},
            )],
            usage=make_usage(input_tokens=200, output_tokens=50),
        ),
        MockResponse(
            stop_reason="end_turn",
            content=[text_block("acknowledged")],
            usage=make_usage(input_tokens=400, output_tokens=80),
        ),
    ])
