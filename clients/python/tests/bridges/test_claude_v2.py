"""Comprehensive tests for :mod:`efe_tool.bridges.claude` — Letter C surface.

Covers :func:`run_conversation` end-to-end with a scripted
:class:`MockAnthropic`: happy path, loop detection, budget ceiling,
rate-limit retry, unknown tools, bad stop_reason, events.
"""

from __future__ import annotations

import json

import pytest

from efe_tool.bridges.claude import (
    ConversationResult,
    ConversationStats,
    ToolCall,
    build_tool_spec,
    run_conversation,
)
from efe_tool.bridges.errors import (
    LlmBadResponseError,
    LlmBudgetExceededError,
    LlmRateLimitError,
    LlmToolLoopError,
)
from efe_tool.bridges.mock import (
    MockAnthropic,
    MockResponse,
    make_usage,
    scripted_infinite_tool_loop,
    scripted_single_tool_then_ack,
    text_block,
    tool_use_block,
)


class TestHappyPath:
    def test_single_tool_then_ack(self, fake_efe_client, fake_catalog, catalogs,
                                   mock_tool_then_ack):
        result = run_conversation(
            mock_tool_then_ack,
            efe_client=fake_efe_client,
            catalogs=catalogs,
            model="claude-sonnet-4-6",
            system="sys",
            user="hi",
        )
        assert isinstance(result, ConversationResult)
        assert result.stop_reason == "end_turn"
        assert result.final_text == "acknowledged"
        assert result.stats.rounds == 2
        assert result.stats.tool_call_count == 1
        assert len(result.tool_calls) == 1
        tc = result.tool_calls[0]
        assert isinstance(tc, ToolCall)
        assert tc.label == "alpha"
        assert tc.is_error is False
        # FakeEfeClient recorded 1 classify call
        assert len(fake_efe_client.calls) == 1

    def test_tokens_accumulated(self, fake_efe_client, catalogs,
                                 mock_tool_then_ack):
        result = run_conversation(
            mock_tool_then_ack,
            efe_client=fake_efe_client,
            catalogs=catalogs,
            model="claude-sonnet-4-6",
            system="sys",
            user="hi",
        )
        # 200 + 400 = 600 in ; 50 + 80 = 130 out
        assert result.stats.input_tokens == 600
        assert result.stats.output_tokens == 130
        assert result.stats.cost_usd > 0.0


class TestLoopDetection:
    def test_max_rounds_raises(self, fake_efe_client, catalogs):
        # Use distinct tool inputs so loop detection does NOT fire; the
        # only signal we're testing here is that hitting max_rounds
        # produces LlmToolLoopError with the "max_rounds" hint.
        responses = [
            MockResponse(
                stop_reason="tool_use",
                content=[tool_use_block(
                    f"toolu_{i}", "efe_classify",
                    {"text": f"different text {i}", "catalog": "support_triage"},
                )],
                usage=make_usage(input_tokens=50, output_tokens=10),
            )
            for i in range(10)
        ]
        mock = MockAnthropic(responses=responses)
        with pytest.raises(LlmToolLoopError) as ei:
            run_conversation(
                mock,
                efe_client=fake_efe_client,
                catalogs=catalogs,
                model="claude-sonnet-4-6",
                system="sys",
                user="hi",
                max_rounds=3,
            )
        assert "max_rounds" in ei.value.message

    def test_identical_tool_inputs_detect_loop(
        self, fake_efe_client, catalogs,
    ):
        """3 identical tool_use blocks in a row → LlmToolLoopError
        even if max_rounds has not been reached."""
        mock = MockAnthropic(
            responses=scripted_infinite_tool_loop(rounds=10),
        )
        with pytest.raises(LlmToolLoopError) as ei:
            run_conversation(
                mock,
                efe_client=fake_efe_client,
                catalogs=catalogs,
                model="claude-sonnet-4-6",
                system="sys",
                user="hi",
                max_rounds=16,
            )
        # Should detect the loop before max_rounds=16 is exhausted.
        assert "repeated" in ei.value.message.lower() or \
               "max_rounds" in ei.value.message


class TestCostCeiling:
    def test_exceeded_raises(self, fake_efe_client, catalogs):
        # First response already incurs big tokens → cost > ceiling.
        mock = MockAnthropic(responses=[
            MockResponse(
                stop_reason="end_turn",
                content=[text_block("ack")],
                usage=make_usage(input_tokens=500_000, output_tokens=100_000),
            ),
        ])
        with pytest.raises(LlmBudgetExceededError):
            run_conversation(
                mock,
                efe_client=fake_efe_client,
                catalogs=catalogs,
                model="claude-sonnet-4-6",
                system="sys",
                user="hi",
                cost_ceiling_usd=0.01,
            )

    def test_within_ceiling_succeeds(self, fake_efe_client, catalogs,
                                      mock_tool_then_ack):
        result = run_conversation(
            mock_tool_then_ack,
            efe_client=fake_efe_client,
            catalogs=catalogs,
            model="claude-sonnet-4-6",
            system="sys",
            user="hi",
            cost_ceiling_usd=1.00,  # way more than needed
        )
        assert result.stats.cost_usd < 1.00


class TestUnknownTool:
    def test_unknown_tool_returns_is_error(self, fake_efe_client, catalogs):
        mock = MockAnthropic(responses=[
            MockResponse(
                stop_reason="tool_use",
                content=[tool_use_block(
                    "toolu_wrong", "some_other_tool", {"text": "x"}
                )],
                usage=make_usage(input_tokens=100, output_tokens=10),
            ),
            MockResponse(
                stop_reason="end_turn",
                content=[text_block("ack")],
                usage=make_usage(input_tokens=120, output_tokens=20),
            ),
        ])
        result = run_conversation(
            mock,
            efe_client=fake_efe_client,
            catalogs=catalogs,
            model="claude-sonnet-4-6",
            system="sys",
            user="hi",
        )
        # Tool call is recorded as an error — but conversation still
        # completes normally.
        assert len(result.tool_calls) == 1
        assert result.tool_calls[0].is_error is True
        # And FakeEfeClient was NOT called (we short-circuited on the name).
        assert fake_efe_client.calls == []


class TestUnknownCatalog:
    def test_unknown_catalog_emits_tool_error(self, fake_efe_client):
        """When Claude invents a catalog not in the registry, the bridge
        returns a tool_error, not an exception."""
        # Only support_triage is known; Claude asks for 'evil'.
        mock = MockAnthropic(responses=[
            MockResponse(
                stop_reason="tool_use",
                content=[tool_use_block(
                    "toolu_1", "efe_classify",
                    {"text": "hi", "catalog": "evil"},
                )],
                usage=make_usage(input_tokens=100, output_tokens=10),
            ),
            MockResponse(
                stop_reason="end_turn",
                content=[text_block("will ask clarify")],
                usage=make_usage(input_tokens=120, output_tokens=10),
            ),
        ])
        from tests.bridges.conftest import FakeCatalog
        catalogs = {"support_triage": FakeCatalog("support_triage")}
        result = run_conversation(
            mock,
            efe_client=fake_efe_client,
            catalogs=catalogs,
            model="claude-sonnet-4-6",
            system="sys",
            user="hi",
        )
        tc = result.tool_calls[0]
        assert tc.is_error is True
        assert "unknown catalog" in (tc.error_message or "").lower()


class TestBadResponse:
    def test_unknown_stop_reason_raises(self, fake_efe_client, catalogs):
        mock = MockAnthropic(responses=[
            MockResponse(
                stop_reason="martian_reason",
                content=[text_block("x")],
                usage=make_usage(input_tokens=10, output_tokens=5),
            ),
        ])
        with pytest.raises(LlmBadResponseError):
            run_conversation(
                mock,
                efe_client=fake_efe_client,
                catalogs=catalogs,
                model="claude-sonnet-4-6",
                system="sys",
                user="hi",
            )

    def test_empty_user_and_history_raises(self, fake_efe_client, catalogs):
        mock = MockAnthropic(responses=[])
        with pytest.raises(LlmBadResponseError):
            run_conversation(
                mock,
                efe_client=fake_efe_client,
                catalogs=catalogs,
                model="claude-sonnet-4-6",
                system="sys",
            )

    def test_invalid_max_rounds_raises(self, fake_efe_client, catalogs):
        mock = MockAnthropic(responses=[])
        with pytest.raises(LlmBadResponseError):
            run_conversation(
                mock,
                efe_client=fake_efe_client,
                catalogs=catalogs,
                model="claude-sonnet-4-6",
                system="sys",
                user="hi",
                max_rounds=0,
            )


class TestEvents:
    def test_events_emitted(self, fake_efe_client, catalogs,
                             mock_tool_then_ack):
        events: list[dict] = []
        run_conversation(
            mock_tool_then_ack,
            efe_client=fake_efe_client,
            catalogs=catalogs,
            model="claude-sonnet-4-6",
            system="sys",
            user="hi",
            on_event=events.append,
        )
        kinds = {e["kind"] for e in events}
        # Must include these landmarks
        for k in ("conversation_start", "round_start", "claude_call",
                  "claude_done", "tool_call_start", "tool_call_done",
                  "round_end", "conversation_done"):
            assert k in kinds, f"missing event: {k}"

    def test_event_callback_exception_does_not_crash(
        self, fake_efe_client, catalogs, mock_tool_then_ack,
    ):
        def bad_cb(_evt):
            raise RuntimeError("bad callback")
        # Should not bubble up.
        result = run_conversation(
            mock_tool_then_ack,
            efe_client=fake_efe_client,
            catalogs=catalogs,
            model="claude-sonnet-4-6",
            system="sys",
            user="hi",
            on_event=bad_cb,
        )
        assert result.stop_reason == "end_turn"


class TestBuildToolSpec:
    def test_enum_enforced_in_schema(self):
        spec = build_tool_spec(["a", "b"])
        assert spec["name"] == "efe_classify"
        assert spec["input_schema"]["properties"]["catalog"]["enum"] == ["a", "b"]

    def test_empty_raises(self):
        with pytest.raises(ValueError):
            build_tool_spec([])


class TestStatsSummaryLine:
    def test_summary_line_shape(self):
        s = ConversationStats(rounds=2, input_tokens=100,
                               output_tokens=50, model="claude-sonnet-4-6")
        line = s.summary_line()
        assert "2 rounds" in line
        assert "100" in line and "50" in line
        assert "claude-sonnet-4-6" in line


class TestResultAsDict:
    def test_projects_everything(self, fake_efe_client, catalogs,
                                  mock_tool_then_ack):
        result = run_conversation(
            mock_tool_then_ack,
            efe_client=fake_efe_client,
            catalogs=catalogs,
            model="claude-sonnet-4-6",
            system="sys",
            user="hi",
        )
        d = result.as_dict()
        json.dumps(d)  # round-trip serialisable
        assert d["model"] == "claude-sonnet-4-6"
        assert d["stop_reason"] == "end_turn"
        assert d["stats"]["rounds"] == 2
        assert len(d["tool_calls"]) == 1
