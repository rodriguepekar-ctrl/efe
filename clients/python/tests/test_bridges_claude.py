"""Unit tests for the Anthropic Claude bridge.

Covers:
    * tool-spec shape (dispatch-safe, policy-free description)
    * dispatch() happy-path and error-path
    * run_conversation_with_tool() tool-call loop with a mocked Anthropic client
    * tool_result contract: JSON body + is_error flag
    * extract_tool_result / extract_final_text helpers
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any
from unittest.mock import MagicMock

import pytest

from efe_tool.bridges.claude import (
    TOOL_NAME,
    build_tool_spec,
    dispatch,
    extract_final_text,
    extract_tool_result,
    run_conversation_with_tool,
)
from efe_tool.catalog import AnchorCatalog
from efe_tool.types import ClassifyResult


# ---------------------------------------------------------------------
# Fake stand-ins for EFE + Anthropic so we never touch the network.
# ---------------------------------------------------------------------


@dataclass
class _FakeBlock:
    """Minimal stand-in for an Anthropic content block object."""

    type: str
    text: str | None = None
    name: str | None = None
    input: dict[str, Any] | None = None
    id: str = "tu_abc"


@dataclass
class _FakeResponse:
    """Minimal stand-in for the Messages.create response."""

    stop_reason: str
    content: list[_FakeBlock] = field(default_factory=list)


def _classify_result(label: str = "fraud", confidence: float = 0.9, reject: bool = False) -> ClassifyResult:
    return ClassifyResult(
        label=label,
        confidence=confidence,
        alignment_delta=0.1,
        rounds=3,
        reject=reject,
        swarm_id="sid",
        request_id="rid",
        text_hash="hash",
        catalog_version="1",
    )


class _FakeEfeClient:
    """Stand-in for EfeClient that never talks over gRPC."""

    def __init__(self, should_fail: bool = False) -> None:
        self.should_fail = should_fail
        self.calls: list[tuple[str, AnchorCatalog]] = []

    def classify(self, text: str, catalog: AnchorCatalog, **_: Any) -> ClassifyResult:
        self.calls.append((text, catalog))
        if self.should_fail:
            raise RuntimeError("simulated EFE failure")
        return _classify_result()


@pytest.fixture
def catalog(tmp_path: Path) -> AnchorCatalog:
    """Minimal, hermetic catalog that loads without network."""
    import hashlib
    import numpy as np

    class _FakeEmbedder:
        def __init__(self, dim: int = 8) -> None:
            self._dim = dim

        def encode(self, text: str) -> np.ndarray:
            v = np.zeros(self._dim, dtype=np.float32)
            h = hashlib.sha256(text.encode("utf-8")).digest()
            v[int.from_bytes(h[:4], "big") % self._dim] = 1.0
            return v

        def encode_batch(self, texts: list[str]) -> np.ndarray:
            return np.stack([self.encode(t) for t in texts])

        def dim(self) -> int:
            return self._dim

        def fingerprint(self) -> str:
            return f"sentence-transformers/test-fake@{self._dim}"

    path = tmp_path / "cat.yaml"
    path.write_text(
        """
apiVersion: efe.mode1/v1
kind: AnchorCatalog
metadata: { name: demo, version: "1.0.0" }
embedder: { family: sentence-transformers, model: test-fake, dim: 8 }
classes:
  - { name: a, canonical_sentences: ["alpha"] }
  - { name: b, canonical_sentences: ["beta"] }
"""
    )
    return AnchorCatalog.load(path, embedder=_FakeEmbedder(dim=8))


# ---------------------------------------------------------------------
# build_tool_spec
# ---------------------------------------------------------------------


def test_tool_spec_has_stable_name_and_enum() -> None:
    spec = build_tool_spec(["demo", "other"])
    assert spec["name"] == TOOL_NAME
    assert spec["input_schema"]["properties"]["catalog"]["enum"] == ["demo", "other"]
    # Reject empty list.
    with pytest.raises(ValueError):
        build_tool_spec([])


def test_tool_spec_description_is_policy_free() -> None:
    """Regression for M1: no hard-coded 0.82 / 0.62 thresholds."""
    spec = build_tool_spec(["demo"])
    desc = spec["description"]
    # These specific numbers used to live in the description; a future
    # catalogue may use very different thresholds, so they must NOT be
    # baked into the tool spec.
    assert "0.82" not in desc
    assert "0.62" not in desc
    # But the semantics of the `reject` flag must still be explained.
    assert "reject" in desc


# ---------------------------------------------------------------------
# dispatch
# ---------------------------------------------------------------------


def test_dispatch_happy_path(catalog: AnchorCatalog) -> None:
    fake = _FakeEfeClient()
    result = dispatch(
        {"text": "hello", "catalog": "demo"},
        efe_client=fake,
        catalogs={catalog.name: catalog},
    )
    assert "error" not in result
    assert result["label"] == "fraud"
    assert fake.calls and fake.calls[0][0] == "hello"


def test_dispatch_rejects_empty_text(catalog: AnchorCatalog) -> None:
    result = dispatch(
        {"text": "", "catalog": "demo"},
        efe_client=_FakeEfeClient(),
        catalogs={catalog.name: catalog},
    )
    assert result == {"error": "text must be a non-empty string"}


def test_dispatch_rejects_unknown_catalog(catalog: AnchorCatalog) -> None:
    result = dispatch(
        {"text": "hi", "catalog": "nope"},
        efe_client=_FakeEfeClient(),
        catalogs={catalog.name: catalog},
    )
    assert "unknown catalog" in result["error"]


def test_dispatch_wraps_efe_exceptions(catalog: AnchorCatalog) -> None:
    result = dispatch(
        {"text": "hi", "catalog": "demo"},
        efe_client=_FakeEfeClient(should_fail=True),
        catalogs={catalog.name: catalog},
    )
    assert "error" in result
    assert "simulated EFE failure" in result["error"]


# ---------------------------------------------------------------------
# run_conversation_with_tool — focuses on the tool_result contract.
# ---------------------------------------------------------------------


def test_tool_result_is_json_with_lowercase_bools(catalog: AnchorCatalog) -> None:
    """Regression for C1: tool_result.content must be JSON, not Python repr."""
    anthropic_mock = MagicMock()

    # Turn 1: Claude emits one tool_use for efe_classify.
    turn1 = _FakeResponse(
        stop_reason="tool_use",
        content=[
            _FakeBlock(
                type="tool_use",
                name=TOOL_NAME,
                input={"text": "hi", "catalog": "demo"},
                id="tu_001",
            )
        ],
    )
    # Turn 2: after receiving the tool_result, Claude ends its turn.
    turn2 = _FakeResponse(
        stop_reason="end_turn",
        content=[_FakeBlock(type="text", text="All good.")],
    )
    anthropic_mock.messages.create.side_effect = [turn1, turn2]

    transcript = run_conversation_with_tool(
        anthropic_mock,
        efe_client=_FakeEfeClient(),
        catalogs={catalog.name: catalog},
        model="claude-sonnet-4-6",
        system="You are a router.",
        user="hi",
    )

    # Pull out the tool_result block we injected.
    user_with_tool_result = transcript[-2]
    assert user_with_tool_result["role"] == "user"
    tool_block = user_with_tool_result["content"][0]
    assert tool_block["type"] == "tool_result"
    assert tool_block["tool_use_id"] == "tu_001"
    # Must be JSON — parseable with `json.loads` and contain lowercase
    # boolean values.
    parsed = json.loads(tool_block["content"])
    assert parsed["label"] == "fraud"
    assert parsed["reject"] is False  # lowercase `false` in JSON.
    # `is_error` must be explicitly False on the happy path.
    assert tool_block["is_error"] is False


def test_tool_result_flags_is_error_on_dispatch_error(catalog: AnchorCatalog) -> None:
    """Regression for C2: error payloads must carry is_error=true."""
    anthropic_mock = MagicMock()
    turn1 = _FakeResponse(
        stop_reason="tool_use",
        content=[
            _FakeBlock(
                type="tool_use",
                name=TOOL_NAME,
                input={"text": "hi", "catalog": "demo"},
                id="tu_002",
            )
        ],
    )
    turn2 = _FakeResponse(
        stop_reason="end_turn",
        content=[_FakeBlock(type="text", text="ack")],
    )
    anthropic_mock.messages.create.side_effect = [turn1, turn2]

    transcript = run_conversation_with_tool(
        anthropic_mock,
        efe_client=_FakeEfeClient(should_fail=True),
        catalogs={catalog.name: catalog},
        model="claude-sonnet-4-6",
        system="...",
        user="hi",
    )

    tool_block = transcript[-2]["content"][0]
    assert tool_block["is_error"] is True
    parsed = json.loads(tool_block["content"])
    assert "error" in parsed


def test_tool_result_flags_unknown_tool(catalog: AnchorCatalog) -> None:
    """If Claude tries to call a tool that isn't ours, flag is_error=true."""
    anthropic_mock = MagicMock()
    turn1 = _FakeResponse(
        stop_reason="tool_use",
        content=[
            _FakeBlock(
                type="tool_use",
                name="other_tool",  # not TOOL_NAME
                input={},
                id="tu_xxx",
            )
        ],
    )
    turn2 = _FakeResponse(
        stop_reason="end_turn",
        content=[_FakeBlock(type="text", text="ok")],
    )
    anthropic_mock.messages.create.side_effect = [turn1, turn2]

    transcript = run_conversation_with_tool(
        anthropic_mock,
        efe_client=_FakeEfeClient(),
        catalogs={catalog.name: catalog},
        model="claude-sonnet-4-6",
        system="...",
        user="hi",
    )
    tool_block = transcript[-2]["content"][0]
    assert tool_block["is_error"] is True
    parsed = json.loads(tool_block["content"])
    assert parsed["error"].startswith("unknown tool")


def test_conversation_supports_history_multi_turn(catalog: AnchorCatalog) -> None:
    """history= lets callers continue a running conversation."""
    anthropic_mock = MagicMock()
    anthropic_mock.messages.create.return_value = _FakeResponse(
        stop_reason="end_turn",
        content=[_FakeBlock(type="text", text="hello again")],
    )
    prior = [
        {"role": "user", "content": "first question"},
        {"role": "assistant", "content": [_FakeBlock(type="text", text="first answer")]},
    ]
    transcript = run_conversation_with_tool(
        anthropic_mock,
        efe_client=_FakeEfeClient(),
        catalogs={catalog.name: catalog},
        model="claude-sonnet-4-6",
        system="...",
        user="second question",
        history=prior,
    )
    roles = [m["role"] for m in transcript]
    assert roles == ["user", "assistant", "user", "assistant"]


def test_run_conversation_does_not_loop_past_max_iter(catalog: AnchorCatalog) -> None:
    """If Claude keeps asking for tools, we stop after max_iter rounds."""
    anthropic_mock = MagicMock()
    anthropic_mock.messages.create.return_value = _FakeResponse(
        stop_reason="tool_use",
        content=[
            _FakeBlock(
                type="tool_use",
                name=TOOL_NAME,
                input={"text": "hi", "catalog": "demo"},
                id="tu_loop",
            )
        ],
    )
    transcript = run_conversation_with_tool(
        anthropic_mock,
        efe_client=_FakeEfeClient(),
        catalogs={catalog.name: catalog},
        model="claude-sonnet-4-6",
        system="...",
        user="hi",
        max_iter=3,
    )
    # 3 rounds = 3 assistant turns + 3 tool_result user turns + 1 initial user = 7
    # (plus whatever history was passed — none here).
    assert anthropic_mock.messages.create.call_count == 3


# ---------------------------------------------------------------------
# extract_tool_result + extract_final_text
# ---------------------------------------------------------------------


def test_extract_tool_result_parses_json() -> None:
    transcript = [
        {"role": "user", "content": "hi"},
        {
            "role": "assistant",
            "content": [
                _FakeBlock(
                    type="tool_use",
                    name=TOOL_NAME,
                    input={"text": "hi", "catalog": "demo"},
                    id="tu_1",
                )
            ],
        },
        {
            "role": "user",
            "content": [
                {
                    "type": "tool_result",
                    "tool_use_id": "tu_1",
                    "is_error": False,
                    "content": json.dumps(
                        {
                            "label": "bug",
                            "confidence": 0.76,
                            "reject": False,
                            "rounds": 4,
                            "alignment_delta": 0.12,
                        }
                    ),
                }
            ],
        },
        {"role": "assistant", "content": [_FakeBlock(type="text", text="Got it.")]},
    ]
    payload = extract_tool_result(transcript)
    assert payload is not None
    assert payload["label"] == "bug"
    assert payload["reject"] is False


def test_extract_tool_result_handles_missing_tool() -> None:
    transcript = [
        {"role": "user", "content": "hi"},
        {"role": "assistant", "content": [_FakeBlock(type="text", text="I answered without the tool.")]},
    ]
    assert extract_tool_result(transcript) is None


def test_extract_tool_result_tolerates_non_json_content() -> None:
    transcript = [
        {
            "role": "user",
            "content": [
                {
                    "type": "tool_result",
                    "tool_use_id": "tu_1",
                    "content": "not-even-json",
                }
            ],
        }
    ]
    payload = extract_tool_result(transcript)
    assert payload == {"raw": "not-even-json"}


def test_extract_final_text_prefers_last_assistant_text() -> None:
    transcript = [
        {"role": "user", "content": "hi"},
        {"role": "assistant", "content": [_FakeBlock(type="text", text="earlier")]},
        {"role": "assistant", "content": [_FakeBlock(type="text", text="final")]},
    ]
    assert extract_final_text(transcript) == "final"


def test_extract_final_text_empty_when_only_tool_use() -> None:
    transcript = [
        {"role": "user", "content": "hi"},
        {
            "role": "assistant",
            "content": [
                _FakeBlock(type="tool_use", name=TOOL_NAME, input={}, id="tu_x"),
            ],
        },
    ]
    assert extract_final_text(transcript) == ""
