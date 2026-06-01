"""StreamMultiplexer translations."""

from __future__ import annotations

from efe_inference.messages.response import FinishReason
from efe_inference.streaming.multiplexer import (
    sse_chunk_to_canonical_openai,
    sse_format_openai,
)


def test_openai_chunk_text_to_canonical():
    raw = {"choices": [{"index": 0, "delta": {"content": "hello"}}]}
    chunk = sse_chunk_to_canonical_openai(raw)
    assert chunk.delta_text == "hello"
    assert chunk.finish_reason is None


def test_openai_chunk_finish_reason():
    raw = {
        "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
    }
    chunk = sse_chunk_to_canonical_openai(raw)
    assert chunk.finish_reason == FinishReason.STOP


def test_openai_chunk_tool_call_delta():
    raw = {"choices": [{"index": 0, "delta": {
        "tool_calls": [{
            "id": "call_1",
            "function": {"name": "get_weather", "arguments": '{"c":"P"}'},
        }],
    }}]}
    chunk = sse_chunk_to_canonical_openai(raw)
    assert chunk.delta_tool_call is not None
    assert chunk.delta_tool_call.name == "get_weather"


def test_sse_format_openai_includes_delta_content():
    from efe_inference.messages.response import ChatChunk

    chunk = ChatChunk(delta_text="hi")
    line = sse_format_openai(chunk, model="mock")
    assert line.startswith("data: ")
    assert "hi" in line
    assert line.endswith("\n\n")
