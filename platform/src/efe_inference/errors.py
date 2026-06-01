"""Exception hierarchy for :mod:`efe_inference`.

Mirrors the pattern of :mod:`efe_registry.errors` / :mod:`efe_gate.errors`:

* single root :class:`InferenceError` for broad trap;
* stable ``code`` per subclass (wire-level compatibility);
* ``retryable_hint`` for clients / daemons;
* ``http_status`` pins REST mapping.

Categories:

    Config / validation
    Request normalisation
    Backend connectivity
    Safety (PII, moderation, injection, policy)
    Cache
    Resilience (circuit, timeout, budget)
    Tool calling
    Streaming
    Accounting
"""

from __future__ import annotations

from typing import Any


class InferenceError(Exception):
    """Root of the efe-inference error hierarchy."""

    code: str = "inference_error"
    retryable_hint: bool = False
    http_status: int = 500

    def __init__(self, message: str, *, detail: Any = None) -> None:
        super().__init__(message)
        self.message = message
        self.detail = detail

    def to_dict(self) -> dict[str, Any]:
        out: dict[str, Any] = {
            "code": self.code,
            "message": self.message,
            "retryable_hint": self.retryable_hint,
        }
        if self.detail is not None:
            out["detail"] = self.detail
        return out

    def __repr__(self) -> str:
        return (
            f"{type(self).__name__}(code={self.code!r}, "
            f"message={self.message!r})"
        )


# ---------------------------------------------------------------------------
# Config / validation
# ---------------------------------------------------------------------------


class ConfigError(InferenceError):
    code = "config_error"
    http_status = 500


class ValidationError(InferenceError):
    code = "validation_error"
    http_status = 400


class TenantMismatch(InferenceError):
    code = "tenant_mismatch"
    http_status = 403


class NotFound(InferenceError):
    code = "not_found"
    http_status = 404


# ---------------------------------------------------------------------------
# Request normalisation
# ---------------------------------------------------------------------------


class RequestError(InferenceError):
    code = "request_error"
    http_status = 400


class UnsupportedMessageRole(RequestError):
    code = "unsupported_role"


class UnsupportedContentKind(RequestError):
    code = "unsupported_content"


class ImageTooLarge(RequestError):
    code = "image_too_large"
    http_status = 413


# ---------------------------------------------------------------------------
# Backend
# ---------------------------------------------------------------------------


class BackendError(InferenceError):
    code = "backend_error"
    http_status = 502
    retryable_hint = True


class BackendUnavailable(BackendError):
    code = "backend_unavailable"
    http_status = 503
    retryable_hint = True


class BackendAuthError(BackendError):
    code = "backend_auth_error"
    http_status = 500
    retryable_hint = False


class BackendRateLimited(BackendError):
    code = "backend_rate_limited"
    http_status = 429
    retryable_hint = True


class BackendBadRequest(BackendError):
    code = "backend_bad_request"
    http_status = 400
    retryable_hint = False


class AllBackendsDown(BackendError):
    """Every backend in the fallback chain returned an error."""

    code = "all_backends_down"
    http_status = 503
    retryable_hint = True


class BackendNotRegistered(InferenceError):
    code = "backend_not_registered"
    http_status = 500


# ---------------------------------------------------------------------------
# Safety
# ---------------------------------------------------------------------------


class SafetyError(InferenceError):
    code = "safety_error"
    http_status = 403


class SafetyBlocked(SafetyError):
    """Raised when a preflight / postflight safety check blocks a request."""

    code = "content_policy_violation"
    http_status = 403


class PiiLeakDetected(SafetyError):
    code = "pii_leak_detected"
    http_status = 500


class ModerationUnavailable(SafetyError):
    code = "moderation_unavailable"
    http_status = 503
    retryable_hint = True


class InjectionSuspected(SafetyBlocked):
    code = "injection_suspected"


# ---------------------------------------------------------------------------
# Cache
# ---------------------------------------------------------------------------


class CacheError(InferenceError):
    code = "cache_error"
    http_status = 500


class EmbeddingUnavailable(CacheError):
    code = "embedding_unavailable"
    http_status = 503
    retryable_hint = True


# ---------------------------------------------------------------------------
# Resilience
# ---------------------------------------------------------------------------


class ResilienceError(InferenceError):
    code = "resilience_error"
    http_status = 500


class CircuitOpen(ResilienceError):
    """Circuit-breaker refused a call."""

    code = "circuit_open"
    http_status = 503
    retryable_hint = True


class DeadlineExceeded(ResilienceError):
    code = "deadline_exceeded"
    http_status = 504
    retryable_hint = True


class BudgetExceeded(ResilienceError):
    """Tenant's daily budget has been exhausted."""

    code = "budget_exceeded"
    http_status = 402


# ---------------------------------------------------------------------------
# Tool calling
# ---------------------------------------------------------------------------


class ToolError(InferenceError):
    code = "tool_error"
    http_status = 400


class ToolArgumentsInvalid(ToolError):
    code = "tool_arguments_invalid"
    http_status = 400


class ToolSchemaInvalid(ToolError):
    code = "tool_schema_invalid"
    http_status = 400


class ToolNotFound(ToolError):
    code = "tool_not_found"
    http_status = 404


# ---------------------------------------------------------------------------
# Streaming
# ---------------------------------------------------------------------------


class StreamError(InferenceError):
    code = "stream_error"
    http_status = 500


class StreamAborted(StreamError):
    code = "stream_aborted"


# ---------------------------------------------------------------------------
# Accounting
# ---------------------------------------------------------------------------


class AccountingError(InferenceError):
    code = "accounting_error"
    http_status = 500


class TokenizerUnavailable(AccountingError):
    code = "tokenizer_unavailable"
    retryable_hint = True


class CostUnknown(AccountingError):
    code = "cost_unknown"


__all__ = [
    "AccountingError",
    "AllBackendsDown",
    "BackendAuthError",
    "BackendBadRequest",
    "BackendError",
    "BackendNotRegistered",
    "BackendRateLimited",
    "BackendUnavailable",
    "BudgetExceeded",
    "CacheError",
    "CircuitOpen",
    "ConfigError",
    "CostUnknown",
    "DeadlineExceeded",
    "EmbeddingUnavailable",
    "ImageTooLarge",
    "InferenceError",
    "InjectionSuspected",
    "ModerationUnavailable",
    "NotFound",
    "PiiLeakDetected",
    "RequestError",
    "ResilienceError",
    "SafetyBlocked",
    "SafetyError",
    "StreamAborted",
    "StreamError",
    "TenantMismatch",
    "TokenizerUnavailable",
    "ToolArgumentsInvalid",
    "ToolError",
    "ToolNotFound",
    "ToolSchemaInvalid",
    "UnsupportedContentKind",
    "UnsupportedMessageRole",
    "ValidationError",
]
