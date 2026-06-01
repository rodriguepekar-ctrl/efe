"""Exception hierarchy for :mod:`efe_gate`.

Design principles (mirrors the rest of the EFE platform):

* Single root :class:`GateError` — one clause traps anything.
* Every subclass carries a :attr:`code` for audit logs and metric
  labels; every code is stable wire-level.
* :attr:`retryable_hint` surfaces a best-guess for clients / the
  orchestrator retry loop.
* :attr:`http_status` pins the HTTP mapping the API layer uses.

Categories:

    Config / validation    — user-supplied inputs
    State machine          — illegal transitions
    Golden set             — immutability violations
    Metrics / stats        — computation errors
    Decision               — rule/engine failures
    Champion / registry    — promotion + rollback consistency
    Runner                 — eval job infrastructure errors
    Audit chain            — signing + integrity failures
    Review                 — human-review flow
"""

from __future__ import annotations

from typing import Any


class GateError(Exception):
    """Root of the efe-gate error hierarchy."""

    code: str = "gate_error"
    retryable_hint: bool = False
    http_status: int = 500

    def __init__(self, message: str, *, detail: Any = None) -> None:
        super().__init__(message)
        self.message = message
        self.detail = detail

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "code": self.code,
            "message": self.message,
            "retryable_hint": self.retryable_hint,
        }
        if self.detail is not None:
            payload["detail"] = self.detail
        return payload

    def __repr__(self) -> str:
        return (
            f"{type(self).__name__}(code={self.code!r}, "
            f"message={self.message!r})"
        )


# ---------------------------------------------------------------------------
# Configuration / validation
# ---------------------------------------------------------------------------


class ConfigError(GateError):
    code = "config_error"
    http_status = 500


class ValidationError(GateError):
    code = "validation_error"
    http_status = 400


class TenantMismatch(GateError):
    code = "tenant_mismatch"
    http_status = 403


class NotFound(GateError):
    code = "not_found"
    http_status = 404


# ---------------------------------------------------------------------------
# State machine
# ---------------------------------------------------------------------------


class StateMachineError(GateError):
    code = "fsm_error"
    http_status = 409


class InvalidTransition(StateMachineError):
    code = "invalid_transition"
    http_status = 409


class TerminalStateReached(StateMachineError):
    code = "terminal_state"
    http_status = 409


# ---------------------------------------------------------------------------
# Golden sets
# ---------------------------------------------------------------------------


class GoldenSetError(GateError):
    code = "golden_set_error"
    http_status = 400


class GoldenSetLocked(GoldenSetError):
    """Attempt to mutate a locked (content-hashed) golden set."""

    code = "golden_set_locked"
    http_status = 409


class GoldenSetDeprecated(GoldenSetError):
    code = "golden_set_deprecated"
    http_status = 409


class GoldenSetChecksumMismatch(GoldenSetError):
    code = "golden_set_checksum_mismatch"
    http_status = 422


# ---------------------------------------------------------------------------
# Metrics / statistical tests
# ---------------------------------------------------------------------------


class MetricsError(GateError):
    code = "metrics_error"
    http_status = 400


class InsufficientSample(MetricsError):
    code = "insufficient_sample"
    http_status = 422


class UnknownMetric(MetricsError):
    code = "unknown_metric"
    http_status = 400


class StatisticalTestError(MetricsError):
    code = "stat_test_error"
    http_status = 500


# ---------------------------------------------------------------------------
# Decision
# ---------------------------------------------------------------------------


class DecisionError(GateError):
    code = "decision_error"
    http_status = 500


class RuleParseError(DecisionError):
    code = "rule_parse_error"
    http_status = 400


class MissingRule(DecisionError):
    code = "missing_rule"
    http_status = 404


# ---------------------------------------------------------------------------
# Champion / registry
# ---------------------------------------------------------------------------


class RegistryError(GateError):
    code = "registry_error"
    http_status = 500


class ConcurrentPromotion(RegistryError):
    """Two promotes raced on the same (tenant, task)."""

    code = "concurrent_promotion"
    http_status = 409
    retryable_hint = True


class NoChampion(RegistryError):
    """No champion yet for a given (tenant, task)."""

    code = "no_champion"
    http_status = 404


# ---------------------------------------------------------------------------
# Rollback
# ---------------------------------------------------------------------------


class RollbackError(GateError):
    code = "rollback_error"
    http_status = 500


class RollbackWindowExpired(RollbackError):
    code = "rollback_window_expired"
    http_status = 409


class NoPreviousChampion(RollbackError):
    """Rollback requested but no previous champion to fall back to."""

    code = "no_previous_champion"
    http_status = 409


# ---------------------------------------------------------------------------
# Runner (eval job)
# ---------------------------------------------------------------------------


class RunnerError(GateError):
    code = "runner_error"
    http_status = 500
    retryable_hint = True


class ArtifactFetchFailed(RunnerError):
    code = "artifact_fetch_failed"
    retryable_hint = True


class RunnerTimeout(RunnerError):
    code = "runner_timeout"
    retryable_hint = True


# ---------------------------------------------------------------------------
# Audit chain
# ---------------------------------------------------------------------------


class AuditChainError(GateError):
    code = "audit_chain_error"
    http_status = 500


class ChainIntegrityBroken(AuditChainError):
    code = "chain_integrity_broken"
    http_status = 500


class SignatureError(AuditChainError):
    code = "signature_error"
    http_status = 500


# ---------------------------------------------------------------------------
# Review
# ---------------------------------------------------------------------------


class ReviewError(GateError):
    code = "review_error"
    http_status = 409


class DualSignoffIncomplete(ReviewError):
    code = "dual_signoff_incomplete"
    http_status = 409


class ReviewTimeout(ReviewError):
    code = "review_timeout"
    http_status = 408


class UnknownReviewer(ReviewError):
    code = "unknown_reviewer"
    http_status = 403


__all__ = [
    "ArtifactFetchFailed",
    "AuditChainError",
    "ChainIntegrityBroken",
    "ConcurrentPromotion",
    "ConfigError",
    "DecisionError",
    "DualSignoffIncomplete",
    "GateError",
    "GoldenSetChecksumMismatch",
    "GoldenSetDeprecated",
    "GoldenSetError",
    "GoldenSetLocked",
    "InsufficientSample",
    "InvalidTransition",
    "MetricsError",
    "MissingRule",
    "NoChampion",
    "NoPreviousChampion",
    "NotFound",
    "RegistryError",
    "ReviewError",
    "ReviewTimeout",
    "RollbackError",
    "RollbackWindowExpired",
    "RuleParseError",
    "RunnerError",
    "RunnerTimeout",
    "SignatureError",
    "StateMachineError",
    "StatisticalTestError",
    "TenantMismatch",
    "TerminalStateReached",
    "UnknownMetric",
    "UnknownReviewer",
    "ValidationError",
]
