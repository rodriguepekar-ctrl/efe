"""Tests for :mod:`efe_tool.bridges.mock`."""

from __future__ import annotations

import pytest

from efe_tool.bridges.mock import (
    MockAnthropic,
    MockResponse,
    make_usage,
    scripted_infinite_tool_loop,
    scripted_single_tool_then_ack,
    text_block,
    tool_use_block,
)


class TestQueue:
    def test_pops_in_order(self):
        r1 = MockResponse(stop_reason="end_turn",
                          content=[text_block("a")])
        r2 = MockResponse(stop_reason="end_turn",
                          content=[text_block("b")])
        m = MockAnthropic(responses=[r1, r2])
        assert m.messages.create(model="x", messages=[]) is r1
        assert m.messages.create(model="x", messages=[]) is r2

    def test_exhausted_queue_raises(self):
        m = MockAnthropic(responses=[])
        with pytest.raises(AssertionError):
            m.messages.create(model="x", messages=[])

    def test_records_calls(self):
        r = MockResponse(stop_reason="end_turn", content=[text_block("a")])
        m = MockAnthropic(responses=[r])
        m.messages.create(model="m", messages=[{"a": 1}])
        assert len(m.calls) == 1
        assert m.calls[0]["model"] == "m"

    def test_enqueue_appends(self):
        m = MockAnthropic(responses=[])
        m.enqueue(MockResponse(stop_reason="end_turn"))
        assert m.messages.create(model="x", messages=[]).stop_reason == "end_turn"

    def test_raise_exception(self):
        class _Boom(Exception): pass
        r = MockResponse(raise_exception=_Boom("nope"))
        m = MockAnthropic(responses=[r])
        with pytest.raises(_Boom):
            m.messages.create(model="x", messages=[])


class TestHelpers:
    def test_text_block(self):
        b = text_block("hello")
        assert b.type == "text"
        assert b.text == "hello"

    def test_tool_use_block(self):
        b = tool_use_block("tid", "efe_classify", {"a": 1})
        assert b.type == "tool_use"
        assert b.id == "tid"
        assert b.name == "efe_classify"
        assert b.input == {"a": 1}

    def test_make_usage(self):
        u = make_usage(input_tokens=5, output_tokens=3)
        assert u.input_tokens == 5
        assert u.output_tokens == 3


class TestConveniences:
    def test_single_tool_then_ack_length(self):
        resps = scripted_single_tool_then_ack(catalog="cat", text="hi")
        assert len(resps) == 2
        assert resps[0].stop_reason == "tool_use"
        assert resps[1].stop_reason == "end_turn"

    def test_infinite_tool_loop_has_many_responses(self):
        resps = scripted_infinite_tool_loop(rounds=15)
        assert len(resps) == 15
        for r in resps:
            assert r.stop_reason == "tool_use"


class TestOnCreate:
    def test_on_create_hook_is_called(self):
        captured: list[dict] = []
        m = MockAnthropic(
            responses=[MockResponse(stop_reason="end_turn")],
            on_create=lambda kw: captured.append(kw),
        )
        m.messages.create(model="abc", messages=[])
        assert len(captured) == 1
        assert captured[0]["model"] == "abc"
