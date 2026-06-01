"""Exception hierarchy for :mod:`efe_rollout`.

All errors descend from a single :class:`RolloutError` root so one
``except`` clause traps everything raised by the module. Every subclass
carries:

* a stable ``code`` (machine-readable, used in audit logs / metric
  labels),
* an ``http_status`` default (pins the REST mapping),
* a ``retryable_hint`` boolean (best-guess for clients / daemons).

Categories (in order of appearance):

    Config               — configuration / startup errors
    Experiment FSM       — invalid state transitions
    Pre-registration     — HMAC-sealed plan failures
    Assignment           — forced / holdout / hash-based routing
    Layer                — orthogonality / slot contention
    Exposure / Outcome   — ingestion / signature
    Analysis             — statistical pipeline failures
    Bandit               — MAB state mismatches
    Guardrail            — auto-abort flow
    NATS                 — eventing integrity
    Integration          — external-module (M05/M06/M07) failures
"""

from __future__ import annotations

from typing import Any


class RolloutError(Exception):
    """Root of the :mod:`efe_rollout` error hierarchy.

    Carries a stable ``code``, an ``http_status`` default and a
    ``retryable_hint``. The :meth:`to_dict` helper is consumed by the
    FastAPI exception handler in ``api/http_app.py``.
    """

    code: str = "rollout_error"
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
# Configuration
# ---------------------------------------------------------------------------


class ConfigError(RolloutError):
    code = "config_error"
    http_status = 500


class ValidationError(RolloutError):
    code = "validation_error"
    http_status = 400


class NotFound(RolloutError):
    code = "not_found"
    http_status = 404


class TenantMismatch(RolloutError):
    code = "tenant_mismatch"
    http_status = 403


# ---------------------------------------------------------------------------
# Experiment FSM
# ---------------------------------------------------------------------------


class ExperimentError(RolloutError):
    code = "experiment_error"
    http_status = 409


class ExperimentNotFound(ExperimentError):
    code = "experiment_not_found"
    http_status = 404


class DuplicateExperiment(ExperimentError):
    code = "duplicate_experiment"
    http_status = 409


class InvalidTransition(ExperimentError):
    code = "invalid_transition"
    http_status = 409


class StateVersionConflict(ExperimentError):
    """Optimistic-lock (CAS) failure — refetch and retry."""

    code = "state_version_conflict"
    http_status = 409
    retryable_hint = True


# ---------------------------------------------------------------------------
# Pre-registration
# ---------------------------------------------------------------------------


class PreRegistrationError(RolloutError):
    code = "prereg_error"
    http_status = 409


class PreRegistrationMissing(PreRegistrationError):
    code = "prereg_missing"
    http_status = 409


class PreRegistrationSealed(PreRegistrationError):
    """Cannot mutate a spec whose pre-registration has been sealed."""

    code = "prereg_sealed"
    http_status = 409


class PreRegistrationSignatureInvalid(PreRegistrationError):
    code = "prereg_signature_invalid"
    http_status = 409


class PreRegistrationSpecHashMismatch(PreRegistrationError):
    code = "prereg_spec_hash_mismatch"
    http_status = 409


# ---------------------------------------------------------------------------
# Assignment
# ---------------------------------------------------------------------------


class AssignmentError(RolloutError):
    code = "assignment_error"
    http_status = 500


class ExperimentNotRunning(AssignmentError):
    code = "experiment_not_running"
    http_status = 409


class NoVariantsDefined(AssignmentError):
    code = "no_variants_defined"
    http_status = 409


class AllocationInvariantBroken(AssignmentError):
    """The sum of variant allocations is not 100 %."""

    code = "allocation_invariant_broken"
    http_status = 500


# ---------------------------------------------------------------------------
# Layers
# ---------------------------------------------------------------------------


class LayerError(RolloutError):
    code = "layer_error"
    http_status = 409


class UnknownLayer(LayerError):
    code = "unknown_layer"
    http_status = 400


class LayerSlotBusy(LayerError):
    """Another experiment already owns ``(tenant, task, layer)``."""

    code = "layer_slot_busy"
    http_status = 409


# ---------------------------------------------------------------------------
# Ingestion (exposures / outcomes)
# ---------------------------------------------------------------------------


class IngestionError(RolloutError):
    code = "ingestion_error"
    http_status = 500


class ExposureSignatureInvalid(IngestionError):
    code = "exposure_signature_invalid"
    http_status = 401


class OutcomeWithoutExposure(IngestionError):
    """Outcome received for a (user, experiment) with no prior exposure.

    This violates invariant **I5** and must either be rejected (strict
    ITT mode) or recorded with a flag (permissive mode).
    """

    code = "outcome_without_exposure"
    http_status = 409


# ---------------------------------------------------------------------------
# Analysis
# ---------------------------------------------------------------------------


class AnalysisError(RolloutError):
    code = "analysis_error"
    http_status = 500


class InsufficientData(AnalysisError):
    code = "insufficient_data"
    http_status = 409


class UnknownMetric(AnalysisError):
    code = "unknown_metric"
    http_status = 404


class UnknownMethod(AnalysisError):
    code = "unknown_method"
    http_status = 400


# ---------------------------------------------------------------------------
# Bandit
# ---------------------------------------------------------------------------


class BanditError(RolloutError):
    code = "bandit_error"
    http_status = 500


class BanditStateNotFound(BanditError):
    code = "bandit_state_not_found"
    http_status = 404


class BanditInvariantBroken(BanditError):
    """Sum of arm pulls or reward counters got corrupted."""

    code = "bandit_invariant_broken"
    http_status = 500


# ---------------------------------------------------------------------------
# Guardrails
# ---------------------------------------------------------------------------


class GuardrailError(RolloutError):
    code = "guardrail_error"
    http_status = 500


class GuardrailBreached(GuardrailError):
    code = "guardrail_breached"
    http_status = 409


class AutoAbortFailed(GuardrailError):
    """Upstream (M06.Rollback.execute()) failed during auto-abort."""

    code = "auto_abort_failed"
    http_status = 500
    retryable_hint = True


# ---------------------------------------------------------------------------
# NATS
# ---------------------------------------------------------------------------


class NatsError(RolloutError):
    code = "nats_error"
    http_status = 500
    retryable_hint = True


class NatsMessageIntegrity(NatsError):
    code = "nats_message_integrity"
    http_status = 401


class NatsEnvelopeStale(NatsError):
    """Envelope timestamp outside the skew window."""

    code = "nats_envelope_stale"
    http_status = 401


# ---------------------------------------------------------------------------
# Integration with adjacent modules
# ---------------------------------------------------------------------------


class IntegrationError(RolloutError):
    code = "integration_error"
    http_status = 502
    retryable_hint = True


class RegistryUnavailable(IntegrationError):
    """M06 rollback endpoint is down."""

    code = "registry_unavailable"
    http_status = 503


class InferenceUnavailable(IntegrationError):
    """M07 inference completed topic isn't producing."""

    code = "inference_unavailable"
    http_status = 503


__all__ = [
    "AllocationInvariantBroken",
    "AnalysisError",
    "AssignmentError",
    "AutoAbortFailed",
    "BanditError",
    "BanditInvariantBroken",
    "BanditStateNotFound",
    "ConfigError",
    "DuplicateExperiment",
    "ExperimentError",
    "ExperimentNotFound",
    "ExperimentNotRunning",
    "ExposureSignatureInvalid",
    "GuardrailBreached",
    "GuardrailError",
    "InferenceUnavailable",
    "IngestionError",
    "InsufficientData",
    "IntegrationError",
    "InvalidTransition",
    "LayerError",
    "LayerSlotBusy",
    "NatsEnvelopeStale",
    "NatsError",
    "NatsMessageIntegrity",
    "NoVariantsDefined",
    "NotFound",
    "OutcomeWithoutExposure",
    "PreRegistrationError",
    "PreRegistrationMissing",
    "PreRegistrationSealed",
    "PreRegistrationSignatureInvalid",
    "PreRegistrationSpecHashMismatch",
    "RegistryUnavailable",
    "RolloutError",
    "StateVersionConflict",
    "TenantMismatch",
    "UnknownLayer",
    "UnknownMethod",
    "UnknownMetric",
    "ValidationError",
]
