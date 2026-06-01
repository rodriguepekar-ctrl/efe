"""Tests for :mod:`efe_tool.scenarios.transcript`."""

from __future__ import annotations

import json
from pathlib import Path
from types import SimpleNamespace

import pytest

from efe_tool.scenarios.transcript import (
    TRANSCRIPT_SCHEMA_VERSION,
    build_transcript,
    write_transcript,
)


class _FakeStats:
    def __init__(self, rounds=1, input_tokens=100, output_tokens=20, cost=0.001):
        self.rounds = rounds
        self.input_tokens = input_tokens
        self.output_tokens = output_tokens
        self.cost_usd = cost
        self.tool_call_count = 1
        self.rate_limit_retries = 0
        self.model = "claude-sonnet-4-6"

    def as_dict(self) -> dict:
        return {
            "rounds": self.rounds,
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "cost_usd": self.cost_usd,
        }


class _FakeToolCall:
    def __init__(self, label="alpha"):
        self.id = "toolu_1"
        self.name = "efe_classify"
        self.round = 1
        self.catalog = "support_triage"
        self.text_length = 10
        self.text_hash = "abc"
        self.label = label
        self.confidence = 0.85
        self.reject = False
        self.is_error = False

    def as_dict(self) -> dict:
        return {"id": self.id, "label": self.label}


class _FakeResult:
    def __init__(self, messages=None):
        self.transcript = messages or []
        self.final_text = "hi"
        self.stats = _FakeStats()
        self.tool_calls = [_FakeToolCall()]
        self.stop_reason = "end_turn"
        self.wall_clock_ms = 1234
        self.model = "claude-sonnet-4-6"


class _FakeScenario:
    def __init__(self):
        self.name = "my_scenario"
        self.prompt = SimpleNamespace(
            version="1.0.0",
            model="claude-sonnet-4-6",
            hash="sha256:abcdef1234567890",
            catalog_names=("support_triage",),
        )


class TestBuildTranscript:
    def test_basic(self):
        result = _FakeResult()
        scenario = _FakeScenario()
        t = build_transcript(
            result=result, scenario=scenario,
            boot_config={"endpoint": "127.0.0.1:50051"},
        )
        assert t["schema_version"] == TRANSCRIPT_SCHEMA_VERSION
        assert t["scenario"]["name"] == "my_scenario"
        assert t["boot_config"]["endpoint"] == "127.0.0.1:50051"
        assert t["stats"]["rounds"] == 1
        assert t["tool_calls"][0]["label"] == "alpha"

    def test_without_scenario(self):
        t = build_transcript(result=_FakeResult())
        assert t["scenario"] == {}

    def test_project_messages_with_text_block(self):
        msg = {"role": "assistant",
               "content": [SimpleNamespace(type="text", text="hello")]}
        t = build_transcript(result=_FakeResult(messages=[msg]))
        assert t["messages"][0]["content"][0]["type"] == "text"
        assert t["messages"][0]["content"][0]["text"] == "hello"

    def test_project_messages_with_tool_use(self):
        block = SimpleNamespace(
            type="tool_use", id="tid", name="efe_classify",
            input={"text": "hi", "catalog": "cat"},
        )
        msg = {"role": "assistant", "content": [block]}
        t = build_transcript(result=_FakeResult(messages=[msg]))
        out = t["messages"][0]["content"][0]
        assert out["type"] == "tool_use"
        assert out["id"] == "tid"
        assert out["input"]["text"] == "hi"

    def test_project_messages_with_tool_result(self):
        block = {"type": "tool_result", "tool_use_id": "tid",
                 "is_error": False, "content": '{"label": "x"}'}
        msg = {"role": "user", "content": [block]}
        t = build_transcript(result=_FakeResult(messages=[msg]))
        out = t["messages"][0]["content"][0]
        assert out["type"] == "tool_result"
        assert out["tool_use_id"] == "tid"
        assert out["content"] == '{"label": "x"}'

    def test_is_json_serialisable(self):
        t = build_transcript(result=_FakeResult(
            messages=[{"role": "user", "content": "hi"}]
        ))
        # Must round-trip through JSON cleanly.
        json.dumps(t)


class TestRedactUser:
    def test_redacts_plain_string_user_content(self):
        msg = {"role": "user", "content": "my super secret"}
        t = build_transcript(result=_FakeResult(messages=[msg]),
                             redact_user=True)
        assert "secret" not in t["messages"][0]["content"]
        assert "redacted" in t["messages"][0]["content"]

    def test_redacts_text_block_in_user_turn(self):
        block = SimpleNamespace(type="text", text="my super secret")
        msg = {"role": "user", "content": [block]}
        t = build_transcript(result=_FakeResult(messages=[msg]),
                             redact_user=True)
        assert "secret" not in t["messages"][0]["content"][0]["text"]

    def test_does_not_redact_assistant(self):
        msg = {"role": "assistant", "content": "my super secret"}
        t = build_transcript(result=_FakeResult(messages=[msg]),
                             redact_user=True)
        assert "secret" in t["messages"][0]["content"]


class TestWriteTranscript:
    def test_atomic_write(self, tmp_path: Path):
        target = tmp_path / "x.json"
        write_transcript({"schema_version": 1}, target)
        assert target.exists()
        assert json.loads(target.read_text(encoding="utf-8"))["schema_version"] == 1
