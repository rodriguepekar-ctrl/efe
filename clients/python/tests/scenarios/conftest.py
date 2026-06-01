"""Shared fixtures for scenario tests."""

from __future__ import annotations

import json
import os
from pathlib import Path
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
    for k in list(os.environ.keys()):
        if k.startswith(("EFE_", "ANTHROPIC_")):
            monkeypatch.delenv(k, raising=False)


@pytest.fixture
def scenario_dir(tmp_path: Path) -> Path:
    """A minimal well-formed scenario under a tmp dir."""
    root = tmp_path / "my_scenario"
    root.mkdir()
    (root / "__init__.py").write_text("", encoding="utf-8")
    (root / "prompt.md").write_text(
        "---\n"
        "name: my_scenario\n"
        "version: 1.0.0\n"
        "model: claude-sonnet-4-6\n"
        "catalogs:\n"
        "  - support_triage\n"
        "---\n"
        "\n"
        "# Role\nYou are a router.\n",
        encoding="utf-8",
    )
    (root / "golden.json").write_text(
        json.dumps([
            {"user": "hello", "expected_label": "alpha", "language": "en"},
            {"user": "bonjour", "expected_label": "alpha", "language": "fr"},
        ]),
        encoding="utf-8",
    )
    (root / "README.md").write_text("# My Scenario\n", encoding="utf-8")
    return root


class FakeCatalog:
    def __init__(self, name: str = "support_triage"):
        self.name = name
        self.version = "1.0.0"
        self.embedder_fingerprint = "sentence-transformers/dummy@4"
        self.dim = 4


class FakeClassifyResult:
    def __init__(self, label: str = "alpha", confidence: float = 0.85):
        self.label = label
        self.confidence = confidence
        self.rounds = 4
        self.reject = False
        self.alignment_delta = 0.1

    def as_tool_result(self) -> dict[str, Any]:
        return {
            "label": self.label,
            "confidence": self.confidence,
            "alignment_delta": self.alignment_delta,
            "rounds": self.rounds,
            "reject": self.reject,
        }


class FakeEfeClient:
    def __init__(self, label: str = "alpha"):
        self._label = label
        self.calls: list = []

    def classify(self, text: str, catalog: Any, **kw: Any) -> Any:
        self.calls.append(text)
        return FakeClassifyResult(label=self._label)

    def close(self) -> None:
        pass


@pytest.fixture
def fake_catalog() -> FakeCatalog:
    return FakeCatalog()


@pytest.fixture
def fake_efe_client() -> FakeEfeClient:
    return FakeEfeClient()


@pytest.fixture
def mock_anthropic_tool_then_ack() -> MockAnthropic:
    """A 2-turn mock: Claude calls efe_classify then replies with
    'ack'. Usable by multiple tests; each test should create its own
    when token counts matter."""
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
            usage=make_usage(input_tokens=300, output_tokens=80),
        ),
    ])
