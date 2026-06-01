"""Tests for :mod:`efe_tool.bridges.errors`."""

from __future__ import annotations

import pytest

from efe_tool.bridges.errors import (
    LLM_EXIT_CODES,
    LlmApiError,
    LlmAuthError,
    LlmBadResponseError,
    LlmBudgetExceededError,
    LlmError,
    LlmRateLimitError,
    LlmToolLoopError,
    from_anthropic_error,
)


class TestExitCodes:
    @pytest.mark.parametrize("cls,expected", [
        (LlmAuthError, 20),
        (LlmApiError, 21),
        (LlmRateLimitError, 22),
        (LlmToolLoopError, 23),
        (LlmBudgetExceededError, 24),
        (LlmBadResponseError, 25),
    ])
    def test_exit_code(self, cls, expected):
        assert cls.exit_code == expected

    def test_codes_disjoint(self):
        codes = {
            LlmAuthError.exit_code, LlmApiError.exit_code,
            LlmRateLimitError.exit_code, LlmToolLoopError.exit_code,
            LlmBudgetExceededError.exit_code,
            LlmBadResponseError.exit_code,
        }
        assert len(codes) == 6

    def test_registry_has_all(self):
        assert set(LLM_EXIT_CODES.keys()) == {20, 21, 22, 23, 24, 25}


class TestBaseClass:
    def test_all_inherit_from_llm_error(self):
        for cls in (LlmAuthError, LlmApiError, LlmRateLimitError,
                    LlmToolLoopError, LlmBudgetExceededError,
                    LlmBadResponseError):
            assert issubclass(cls, LlmError)

    def test_llm_error_default_is_21(self):
        assert LlmError.exit_code == 21
        assert LlmError.error_code == "LLM_API_ERROR"


class TestPayload:
    def test_to_payload_carries_error_code(self):
        e = LlmToolLoopError("loop", hint="tighten prompt",
                             context={"rounds": 8})
        p = e.to_payload()
        assert p["code"] == "LLM_TOOL_LOOP"
        assert p["exit_code"] == 23
        assert p["hint"] == "tighten prompt"
        assert p["context"]["rounds"] == 8

    def test_to_payload_cause_truncated(self):
        src = ValueError("x" * 1000)
        e = LlmRateLimitError("rate", cause=src)
        assert len(e.to_payload()["cause"]["detail"]) <= 500


class TestFromAnthropicError:
    """These tests need the anthropic package because the adapter
    imports it to do ``isinstance`` checks against real classes.
    If the dep is missing we skip rather than fail."""

    @pytest.fixture(autouse=True)
    def _require_anthropic(self):
        pytest.importorskip("anthropic")

    def test_authentication_error_maps_to_auth(self):
        import anthropic
        # AuthenticationError has a complicated ctor; wrap a best-effort instance.
        exc = anthropic.AuthenticationError(
            message="bad key",
            response=None,   # type: ignore
            body=None,
        ) if hasattr(anthropic, "AuthenticationError") else ValueError("bad key")
        mapped = from_anthropic_error(exc)
        if hasattr(anthropic, "AuthenticationError") and isinstance(
            exc, anthropic.AuthenticationError,
        ):
            assert isinstance(mapped, LlmAuthError)
        else:
            assert isinstance(mapped, LlmApiError)

    def test_unknown_exception_maps_to_api_error(self):
        mapped = from_anthropic_error(ValueError("unexpected"))
        assert isinstance(mapped, LlmApiError)
