"""LLM-specific exception taxonomy for the Letter-C bridge.

These exceptions inherit from :class:`efe_tool.cli.errors.CliError` so
they plug directly into the CLI dispatcher's exit-code discipline. Each
subclass declares its own :attr:`exit_code` and :attr:`error_code`.

See ``LETTER_C_ARCHITECTURE.md`` §10 for the full taxonomy.

+-------+--------------------+------------------------+
|  code |  symbol            |  raised when           |
+-------+--------------------+------------------------+
|  20   |  LLM_AUTH          |  API key missing / 401 |
|  21   |  LLM_API_ERROR     |  any other 4xx/5xx     |
|  22   |  LLM_RATE_LIMIT    |  429 after retries     |
|  23   |  LLM_TOOL_LOOP     |  max_rounds exhausted  |
|  24   |  LLM_BUDGET_EXCEED |  cost ceiling tripped  |
|  25   |  LLM_BAD_RESPONSE  |  stop_reason unknown   |
+-------+--------------------+------------------------+
"""

from __future__ import annotations

from typing import Any, Optional

from ..cli.errors import CliError


# ---------------------------------------------------------------------------
#  Root of the LLM family
# ---------------------------------------------------------------------------


class LlmError(CliError):
    """Base class for every Letter-C LLM-integration failure."""

    exit_code: int = 21
    error_code: str = "LLM_API_ERROR"


# ---------------------------------------------------------------------------
#  Subclasses
# ---------------------------------------------------------------------------


class LlmAuthError(LlmError):
    """``ANTHROPIC_API_KEY`` missing or rejected by the provider."""

    exit_code: int = 20
    error_code: str = "LLM_AUTH"


class LlmApiError(LlmError):
    """Any non-auth, non-ratelimit error from the Anthropic SDK."""

    exit_code: int = 21
    error_code: str = "LLM_API_ERROR"


class LlmRateLimitError(LlmError):
    """All retries exhausted on a 429 rate-limit response."""

    exit_code: int = 22
    error_code: str = "LLM_RATE_LIMIT"


class LlmToolLoopError(LlmError):
    """``max_rounds`` reached, or a same-input loop was detected."""

    exit_code: int = 23
    error_code: str = "LLM_TOOL_LOOP"


class LlmBudgetExceededError(LlmError):
    """Running ``cost_ceiling_usd`` was crossed before completion."""

    exit_code: int = 24
    error_code: str = "LLM_BUDGET_EXCEEDED"


class LlmBadResponseError(LlmError):
    """Claude returned a shape we don't know how to consume
    (unknown stop_reason, non-parseable content, etc.)."""

    exit_code: int = 25
    error_code: str = "LLM_BAD_RESPONSE"


# ---------------------------------------------------------------------------
#  Adapter from anthropic SDK errors
# ---------------------------------------------------------------------------


def from_anthropic_error(exc: BaseException) -> LlmError:
    """Translate an anthropic-SDK exception into our taxonomy.

    The import is deferred so a consumer without ``anthropic`` installed
    doesn't pay the cost of the optional dependency.
    """
    try:
        import anthropic as _a  # type: ignore
    except ImportError:  # pragma: no cover
        return LlmApiError(
            f"anthropic SDK unavailable while mapping error: {exc}",
            cause=exc,
        )

    # AuthenticationError → LLM_AUTH (exit 20)
    if isinstance(exc, getattr(_a, "AuthenticationError", ())):
        return LlmAuthError(
            f"Anthropic authentication failed: {exc}",
            hint="check ANTHROPIC_API_KEY (env)",
            cause=exc,
        )

    # PermissionDeniedError — the model/feature is forbidden for this key.
    # Map to LLM_AUTH too (it's an auth-adjacent problem).
    perm_err = getattr(_a, "PermissionDeniedError", None)
    if perm_err is not None and isinstance(exc, perm_err):
        return LlmAuthError(
            f"Anthropic permission denied: {exc}",
            hint="the API key may lack access to this model",
            cause=exc,
        )

    # RateLimitError → LLM_RATE_LIMIT (exit 22)
    if isinstance(exc, getattr(_a, "RateLimitError", ())):
        return LlmRateLimitError(
            f"Anthropic rate-limit exceeded: {exc}",
            hint="retry later, or lower your request rate",
            cause=exc,
        )

    # NotFoundError — usually the model id is wrong.
    if isinstance(exc, getattr(_a, "NotFoundError", ())):
        return LlmApiError(
            f"Anthropic resource not found: {exc}",
            hint="check the model identifier",
            cause=exc,
        )

    # BadRequestError / ValidationError — the request is malformed.
    bad_req = getattr(_a, "BadRequestError", None)
    if bad_req is not None and isinstance(exc, bad_req):
        return LlmApiError(
            f"Anthropic bad request: {exc}",
            hint="review message shapes and tool spec",
            cause=exc,
        )

    # Connection / timeout — network layer.
    for _candidate in (
        getattr(_a, "APIConnectionError", None),
        getattr(_a, "APITimeoutError", None),
    ):
        if _candidate is not None and isinstance(exc, _candidate):
            return LlmApiError(
                f"Anthropic connection issue: {exc}",
                hint="check network connectivity and API base URL",
                cause=exc,
            )

    # Fallback: any remaining anthropic.APIError.
    if isinstance(exc, getattr(_a, "APIError", ())):
        return LlmApiError(
            f"Anthropic API error: {exc}",
            cause=exc,
        )

    # Non-anthropic exception — surface as generic LlmApiError.
    return LlmApiError(f"unexpected error while calling LLM: {exc}", cause=exc)


# ---------------------------------------------------------------------------
#  Registry for testing & exhaustiveness
# ---------------------------------------------------------------------------


LLM_EXIT_CODES: dict[int, str] = {
    LlmAuthError.exit_code: LlmAuthError.error_code,
    LlmApiError.exit_code: LlmApiError.error_code,
    LlmRateLimitError.exit_code: LlmRateLimitError.error_code,
    LlmToolLoopError.exit_code: LlmToolLoopError.error_code,
    LlmBudgetExceededError.exit_code: LlmBudgetExceededError.error_code,
    LlmBadResponseError.exit_code: LlmBadResponseError.error_code,
}


__all__ = [
    "LlmError",
    "LlmAuthError",
    "LlmApiError",
    "LlmRateLimitError",
    "LlmToolLoopError",
    "LlmBudgetExceededError",
    "LlmBadResponseError",
    "from_anthropic_error",
    "LLM_EXIT_CODES",
]
