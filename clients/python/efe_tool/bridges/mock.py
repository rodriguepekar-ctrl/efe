"""Mock Anthropic SDK for offline testing of Letter-C bridges.

Provides :class:`MockAnthropic`, a drop-in replacement for
``anthropic.Anthropic`` that plays back a scripted queue of responses.
Every test that exercises the conversation dispatcher should use this
mock — real API calls are reserved for the opt-in e2e suite.

See ``LETTER_C_ARCHITECTURE.md`` §15.2 for the design.

Usage::

    from efe_tool.bridges.mock import (
        MockAnthropic, MockResponse, text_block, tool_use_block, make_usage,
    )

    mock = MockAnthropic(responses=[
        MockResponse(
            stop_reason="tool_use",
            content=[tool_use_block("toolu_1", "efe_classify",
                                    {"text": "hi", "catalog": "my_cat"})],
            usage=make_usage(input_tokens=100, output_tokens=20),
        ),
        MockResponse(
            stop_reason="end_turn",
            content=[text_block("hello there")],
            usage=make_usage(input_tokens=200, output_tokens=30),
        ),
    ])

    from efe_tool.bridges.claude import run_conversation
    result = run_conversation(mock, ...)
"""

from __future__ import annotations

from dataclasses import dataclass, field
from types import SimpleNamespace
from typing import Any, Callable, Iterable, Optional


# ---------------------------------------------------------------------------
#  Block helpers
# ---------------------------------------------------------------------------


def text_block(text: str) -> SimpleNamespace:
    """Build a Claude ``text`` content block as a SimpleNamespace."""
    return SimpleNamespace(type="text", text=text)


def tool_use_block(
    block_id: str, name: str, input_: dict[str, Any],
) -> SimpleNamespace:
    """Build a Claude ``tool_use`` content block."""
    return SimpleNamespace(type="tool_use", id=block_id, name=name, input=input_)


def make_usage(
    *, input_tokens: int = 0, output_tokens: int = 0,
    cache_creation_input_tokens: int = 0,
    cache_read_input_tokens: int = 0,
) -> SimpleNamespace:
    """Build a ``usage`` object matching the Anthropic SDK shape."""
    return SimpleNamespace(
        input_tokens=input_tokens,
        output_tokens=output_tokens,
        cache_creation_input_tokens=cache_creation_input_tokens,
        cache_read_input_tokens=cache_read_input_tokens,
    )


# ---------------------------------------------------------------------------
#  Response
# ---------------------------------------------------------------------------


@dataclass
class MockResponse:
    """A scripted response for :class:`MockAnthropic`.

    Attributes
    ----------
    stop_reason:
        One of ``end_turn`` / ``tool_use`` / ``max_tokens`` /
        ``stop_sequence``. Unknown values are passed through.
    content:
        List of content blocks (text, tool_use, …). Any object with a
        ``type`` attribute will do — typically :class:`SimpleNamespace`
        built via :func:`text_block` / :func:`tool_use_block`.
    usage:
        ``usage`` object with ``input_tokens``/``output_tokens`` fields.
    raise_exception:
        If set, the mock raises this exception instead of returning
        the response (used to test rate-limit / auth paths).
    model:
        Optional model id to include on the response (for parity with
        real SDK).
    """

    stop_reason: str = "end_turn"
    content: list = field(default_factory=list)
    usage: SimpleNamespace = field(
        default_factory=lambda: make_usage(input_tokens=0, output_tokens=0),
    )
    raise_exception: Optional[BaseException] = None
    model: Optional[str] = None
    id: str = "msg_mock"
    role: str = "assistant"
    type: str = "message"


# ---------------------------------------------------------------------------
#  Mock client
# ---------------------------------------------------------------------------


class MockAnthropic:
    """A drop-in replacement for the ``anthropic.Anthropic`` client.

    Call :meth:`messages.create` the same way you would on the real
    client; the mock pops the next :class:`MockResponse` from its queue
    and returns it (or raises ``raise_exception`` if set).

    Attributes
    ----------
    calls:
        List of ``dict`` capturing every call's kwargs, for assertions.
    """

    def __init__(
        self,
        responses: Iterable[MockResponse] | None = None,
        *,
        on_create: Optional[Callable[[dict], None]] = None,
    ) -> None:
        self._queue: list[MockResponse] = list(responses or [])
        self.calls: list[dict] = []
        self._on_create = on_create
        self.messages = _MockMessages(self)

    def enqueue(self, *responses: MockResponse) -> None:
        """Append responses to the queue (convenience)."""
        self._queue.extend(responses)

    def _next_response(self, kwargs: dict) -> MockResponse:
        self.calls.append(kwargs)
        if self._on_create is not None:
            self._on_create(kwargs)
        if not self._queue:
            raise AssertionError(
                "MockAnthropic: queue exhausted — "
                f"call #{len(self.calls)} has no scripted response"
            )
        resp = self._queue.pop(0)
        if resp.raise_exception is not None:
            raise resp.raise_exception
        if resp.model is None:
            resp.model = kwargs.get("model")
        return resp


class _MockMessages:
    """Mirrors ``anthropic.Anthropic.messages`` (only ``create`` is defined)."""

    def __init__(self, parent: MockAnthropic) -> None:
        self._parent = parent

    def create(self, **kwargs: Any) -> MockResponse:
        return self._parent._next_response(kwargs)


# ---------------------------------------------------------------------------
#  Convenience builders for common scenarios
# ---------------------------------------------------------------------------


def scripted_single_tool_then_ack(
    *,
    catalog: str = "support_triage",
    text: str = "hello",
    label_for_ack: str = "question",
    input_tokens_per_call: int = 100,
    output_tokens_per_call: int = 30,
) -> list[MockResponse]:
    """Canonical 2-round scenario: tool_use → tool_result → end_turn.

    Handy one-liner for tests that just need to exercise the happy path.
    """
    return [
        MockResponse(
            stop_reason="tool_use",
            content=[tool_use_block("toolu_1", "efe_classify",
                                    {"text": text, "catalog": catalog})],
            usage=make_usage(
                input_tokens=input_tokens_per_call,
                output_tokens=output_tokens_per_call,
            ),
        ),
        MockResponse(
            stop_reason="end_turn",
            content=[text_block(f"ack for {label_for_ack}")],
            usage=make_usage(
                input_tokens=input_tokens_per_call * 2,
                output_tokens=output_tokens_per_call,
            ),
        ),
    ]


def scripted_infinite_tool_loop(
    *,
    catalog: str = "support_triage",
    text: str = "hello",
    rounds: int = 20,
) -> list[MockResponse]:
    """Queue ``rounds`` identical tool_use responses — triggers
    :class:`LlmToolLoopError` once the bridge hits max_rounds or
    detects the loop."""
    return [
        MockResponse(
            stop_reason="tool_use",
            content=[tool_use_block(f"toolu_{i}", "efe_classify",
                                    {"text": text, "catalog": catalog})],
            usage=make_usage(input_tokens=100, output_tokens=20),
        )
        for i in range(rounds)
    ]


__all__ = [
    "MockAnthropic",
    "MockResponse",
    "text_block",
    "tool_use_block",
    "make_usage",
    "scripted_single_tool_then_ack",
    "scripted_infinite_tool_loop",
]
