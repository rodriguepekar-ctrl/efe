"""Exception hierarchy for :mod:`efe_registry`.

Design mirrors :mod:`efe_gate.errors` for platform consistency:

* single root :class:`RegistryError` so one clause traps anything;
* every subclass carries a stable ``code`` for audit logs and metrics;
* ``retryable_hint`` surfaces a best-guess for clients / daemons;
* ``http_status`` pins the REST mapping.

Categories:

    Config           — configuration / startup errors
    Artifact FSM     — invalid state transitions on artifacts
    Signing          — cosign / KMS / Rekor failures
    Routing          — Redis / Lua / cache issues
    Canary           — canary progression failures
    Rollback         — the SLO-bound rollback flow
    Serving          — vLLM / model-load failures
    Contracts        — contract_hash lookup / sunset failures
    Admission        — K8s policy-controller failures
    NATS             — eventing layer
"""

from __future__ import annotations

from typing import Any


class RegistryError(Exception):
    """Root of the efe-registry error hierarchy."""

    code: str = "registry_error"
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


class ConfigError(RegistryError):
    code = "config_error"
    http_status = 500


class ValidationError(RegistryError):
    code = "validation_error"
    http_status = 400


class TenantMismatch(RegistryError):
    code = "tenant_mismatch"
    http_status = 403


class NotFound(RegistryError):
    code = "not_found"
    http_status = 404


# ---------------------------------------------------------------------------
# Artifact state machine
# ---------------------------------------------------------------------------


class ArtifactError(RegistryError):
    code = "artifact_error"
    http_status = 400


class InvalidArtifactTransition(ArtifactError):
    code = "invalid_artifact_transition"
    http_status = 409


class ArtifactImmutable(ArtifactError):
    code = "artifact_immutable"
    http_status = 409


class ArtifactNotFound(ArtifactError):
    code = "artifact_not_found"
    http_status = 404


class DuplicateArtifact(ArtifactError):
    code = "duplicate_artifact"
    http_status = 409


# ---------------------------------------------------------------------------
# Signing / verification
# ---------------------------------------------------------------------------


class SignatureError(RegistryError):
    code = "signature_error"
    http_status = 500


class SignatureExpired(SignatureError):
    code = "signature_expired"
    http_status = 409


class SignatureMismatch(SignatureError):
    code = "signature_mismatch"
    http_status = 409


class ArtifactVerificationFailed(SignatureError):
    """Raised by :class:`VllmAdapter` or :class:`RuntimeVerifier`."""

    code = "artifact_verification_failed"
    http_status = 409


class KmsUnavailable(SignatureError):
    code = "kms_unavailable"
    retryable_hint = True
    http_status = 503


class RekorUnavailable(SignatureError):
    code = "rekor_unavailable"
    retryable_hint = True
    http_status = 503


class NoSigningMethodConfigured(SignatureError):
    code = "no_signing_method"
    http_status = 500


# ---------------------------------------------------------------------------
# Routing / Redis
# ---------------------------------------------------------------------------


class RoutingError(RegistryError):
    code = "routing_error"
    http_status = 500


class RoutingVersionConflict(RoutingError):
    """CAS mismatch on the routing version — caller should refetch + retry."""

    code = "routing_version_conflict"
    http_status = 409
    retryable_hint = True


class RoutingNotFound(RoutingError):
    code = "routing_not_found"
    http_status = 404


class RoutingInvariantBroken(RoutingError):
    """Ex: weights sum != 100, or >1 champion per (tenant,task,variant)."""

    code = "routing_invariant_broken"
    http_status = 500


class QuotaExceeded(RoutingError):
    code = "quota_exceeded"
    http_status = 429
    retryable_hint = True


# ---------------------------------------------------------------------------
# Canary
# ---------------------------------------------------------------------------


class CanaryError(RegistryError):
    code = "canary_error"
    http_status = 409


class CanaryAlreadyActive(CanaryError):
    code = "canary_already_active"
    http_status = 409


class CanaryNotFound(CanaryError):
    code = "canary_not_found"
    http_status = 404


class CanarySloBreach(CanaryError):
    code = "canary_slo_breach"
    http_status = 409


# ---------------------------------------------------------------------------
# Rollback
# ---------------------------------------------------------------------------


class RollbackError(RegistryError):
    code = "rollback_error"
    http_status = 500


class RollbackTimeout(RollbackError):
    code = "rollback_timeout"
    http_status = 500


class NoStandbyAvailable(RollbackError):
    code = "no_standby_available"
    http_status = 503
    retryable_hint = True


class NoPreviousChampion(RollbackError):
    code = "no_previous_champion"
    http_status = 409


class RollbackVersionConflict(RollbackError):
    code = "rollback_version_conflict"
    http_status = 409
    retryable_hint = True


# ---------------------------------------------------------------------------
# Serving / vLLM
# ---------------------------------------------------------------------------


class ServingError(RegistryError):
    code = "serving_error"
    http_status = 500


class ModelLoadFailed(ServingError):
    code = "model_load_failed"
    http_status = 500


class ModelArtifactDownloadFailed(ServingError):
    code = "model_artifact_download_failed"
    http_status = 502
    retryable_hint = True


# ---------------------------------------------------------------------------
# Contracts
# ---------------------------------------------------------------------------


class ContractError(RegistryError):
    code = "contract_error"
    http_status = 400


class ContractNotFound(ContractError):
    code = "contract_not_found"
    http_status = 404


class ContractSunset(ContractError):
    code = "contract_sunset"
    http_status = 410


class ContractSchemaInvalid(ContractError):
    code = "contract_schema_invalid"
    http_status = 422


# ---------------------------------------------------------------------------
# Admission / policy
# ---------------------------------------------------------------------------


class AdmissionError(RegistryError):
    code = "admission_error"
    http_status = 500


class PolicyDenied(AdmissionError):
    code = "policy_denied"
    http_status = 403


class BreakGlassExpired(AdmissionError):
    code = "break_glass_expired"
    http_status = 409


# ---------------------------------------------------------------------------
# NATS
# ---------------------------------------------------------------------------


class NatsError(RegistryError):
    code = "nats_error"
    http_status = 500
    retryable_hint = True


class NatsAuthFailure(NatsError):
    code = "nats_auth_failure"
    http_status = 401


class NatsMessageIntegrity(NatsError):
    """HMAC check failed on an inbound event."""

    code = "nats_message_integrity"
    http_status = 401


__all__ = [
    "AdmissionError",
    "ArtifactError",
    "ArtifactImmutable",
    "ArtifactNotFound",
    "ArtifactVerificationFailed",
    "BreakGlassExpired",
    "CanaryAlreadyActive",
    "CanaryError",
    "CanaryNotFound",
    "CanarySloBreach",
    "ConfigError",
    "ContractError",
    "ContractNotFound",
    "ContractSchemaInvalid",
    "ContractSunset",
    "DuplicateArtifact",
    "InvalidArtifactTransition",
    "KmsUnavailable",
    "ModelArtifactDownloadFailed",
    "ModelLoadFailed",
    "NatsAuthFailure",
    "NatsError",
    "NatsMessageIntegrity",
    "NoPreviousChampion",
    "NoSigningMethodConfigured",
    "NoStandbyAvailable",
    "NotFound",
    "PolicyDenied",
    "QuotaExceeded",
    "RegistryError",
    "RekorUnavailable",
    "RollbackError",
    "RollbackTimeout",
    "RollbackVersionConflict",
    "RoutingError",
    "RoutingInvariantBroken",
    "RoutingNotFound",
    "RoutingVersionConflict",
    "ServingError",
    "SignatureError",
    "SignatureExpired",
    "SignatureMismatch",
    "TenantMismatch",
    "ValidationError",
]
