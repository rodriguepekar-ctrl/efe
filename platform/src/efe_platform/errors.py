"""Exception hierarchy for :mod:`efe_platform`.

Design principles
-----------------

* **Single root** — every exception raised by this package descends from
  :class:`EfePlatformError`. Callers can therefore do a single
  ``except EfePlatformError`` to trap platform-originated failures
  without accidentally swallowing ``KeyboardInterrupt`` / ``SystemExit``.

* **Narrow subclasses** — each failure mode has its own class so
  ``except`` clauses can be precise. The class names intentionally
  mirror the layer in which they originate (``Bot*``, ``Swarm*``,
  ``Tool*``, ``Budget*``, ``Audit*``, ``Config*``).

* **Retryable marker** — a boolean class attribute :attr:`retryable`
  signals whether the failure is worth retrying under a backoff
  policy. The :mod:`efe_platform.swarm.client` retry machinery
  inspects this attribute rather than hard-coding a list.

* **Machine-readable payload** — every instance exposes a
  :meth:`to_dict` method returning a JSON-safe description. This
  drives HTTP 5xx bodies, structured logs and audit entries.
"""

from __future__ import annotations

from typing import Any


class EfePlatformError(Exception):
    """Root of the platform-specific error hierarchy."""

    #: Whether a caller's automatic retry loop should consider retrying
    #: after catching this exception. Overridden by subclasses.
    retryable: bool = False

    #: Short machine-stable error code (``snake_case``) for logs/audit.
    code: str = "efe_platform_error"

    def __init__(self, message: str, *, detail: Any = None) -> None:
        super().__init__(message)
        self.message = message
        self.detail = detail

    # ------------------------------------------------------------------ API

    def to_dict(self) -> dict[str, Any]:
        """JSON-safe rendering used by audit, logs, and HTTP responses."""
        payload: dict[str, Any] = {
            "code": self.code,
            "message": self.message,
            "retryable": self.retryable,
        }
        if self.detail is not None:
            payload["detail"] = self.detail
        return payload

    # ------------------------------------------------------------- Repr

    def __repr__(self) -> str:
        # Keep the repr copy-paste-friendly for tests.
        return f"{type(self).__name__}(code={self.code!r}, message={self.message!r})"


# ---------------------------------------------------------------------------
#  Configuration
# ---------------------------------------------------------------------------


class ConfigError(EfePlatformError):
    """Platform-level configuration is invalid or missing."""

    code = "config_error"
    retryable = False


class LlmProviderUnavailable(ConfigError):
    """No LLM provider backing is wired up — refuse to start the bot."""

    code = "llm_provider_unavailable"


# ---------------------------------------------------------------------------
#  Bot layer
# ---------------------------------------------------------------------------


class BotError(EfePlatformError):
    """Any failure inside the orchestration loop."""

    code = "bot_error"


class PlanError(BotError):
    """The LLM's plan could not be parsed or failed validation."""

    code = "plan_error"


class VerifyError(BotError):
    """The LLM's verify step produced an unparsable or inconsistent output."""

    code = "verify_error"


class BudgetExceeded(BotError):
    """The planned or observed cost exceeds the caller-supplied budget."""

    code = "budget_exceeded"
    retryable = False


# ---------------------------------------------------------------------------
#  Tool layer
# ---------------------------------------------------------------------------


class ToolInvocationError(BotError):
    """A tool-use instruction could not be executed."""

    code = "tool_invocation_error"
    retryable = True  # transient network / deadline
    # Subclasses override this when the cause is definitely not transient.


class UnknownTool(ToolInvocationError):
    """The LLM asked for a tool that is not registered — hard stop."""

    code = "unknown_tool"
    retryable = False


class ToolArgsInvalid(ToolInvocationError):
    """The LLM produced arguments that fail schema validation — hard stop."""

    code = "tool_args_invalid"
    retryable = False


# ---------------------------------------------------------------------------
#  Swarm (gRPC to efe-server)
# ---------------------------------------------------------------------------


class SwarmError(EfePlatformError):
    """Any failure returned by the Rust efe-server."""

    code = "swarm_error"


class SwarmUnavailable(SwarmError):
    """Network or handshake failure — legitimate to retry."""

    code = "swarm_unavailable"
    retryable = True


class SwarmDeadlineExceeded(SwarmError):
    """Deadline propagation triggered — caller may retry with more budget."""

    code = "swarm_deadline_exceeded"
    retryable = True


class SwarmInvalidSpec(SwarmError):
    """The server rejected the SwarmSpec at ``DescribeSpec`` time."""

    code = "swarm_invalid_spec"
    retryable = False


class SwarmDivergence(SwarmError):
    """Swarm returned a DIVERGENCE or MODE_COLLAPSE verdict."""

    code = "swarm_divergence"
    # Technically idempotent-retryable, but the same spec will likely diverge
    # again; leave this off to force the caller to adjust the spec.
    retryable = False


# ---------------------------------------------------------------------------
#  Audit
# ---------------------------------------------------------------------------


class AuditError(EfePlatformError):
    """Audit write failure — e.g. the log file is read-only."""

    code = "audit_error"
    retryable = False


__all__ = [
    "AuditError",
    "BotError",
    "BudgetExceeded",
    "ConfigError",
    "EfePlatformError",
    "LlmProviderUnavailable",
    "PlanError",
    "SwarmDeadlineExceeded",
    "SwarmDivergence",
    "SwarmError",
    "SwarmInvalidSpec",
    "SwarmUnavailable",
    "ToolArgsInvalid",
    "ToolInvocationError",
    "UnknownTool",
    "VerifyError",
]
