"""Exception taxonomy for :mod:`efe_security` — the M11 trust plane.

Mirrors §20 of ``M11_SECURITY_ARCHITECTURE.md``. Every exception carries a
stable ``code``, an ``http_status`` default and a ``retryable_hint`` for
clients. All descend from :class:`SecurityError` so one ``except`` trap
catches the whole module.

The taxonomy is **flat-but-grouped** for operational reasons : a SIEM
filter on ``code == "authz.scope_missing"`` must stay stable across
refactors.
"""

from __future__ import annotations

from typing import Any

__all__ = [
    "AbacConditionFailed",
    "ActionForbidden",
    "AlgorithmDeprecated",
    "AuditError",
    "AuthNError",
    "AuthZError",
    "ChainBroken",
    "ConsentError",
    "ConsentExpired",
    "ConsentMissing",
    "ConsentWithdrawn",
    "CryptoError",
    "DsarError",
    "ErasureBlocked",
    "ExpiredToken",
    "ExportDenied",
    "FieldNotClassified",
    "IncidentError",
    "InvalidAudience",
    "InvalidIssuer",
    "InvalidKid",
    "InvalidSignature",
    "KeyExpired",
    "KeyNotFound",
    "LegalHoldInForce",
    "LifecycleError",
    "MfaRequired",
    "MissingToken",
    "NotificationBlocked",
    "OwnershipViolation",
    "PiiError",
    "PlaybookFailed",
    "PolicyViolation",
    "PurposeNotDeclared",
    "QuotaExceeded",
    "RequestAlreadyProcessed",
    "RetentionConflict",
    "RotationFailed",
    "ScopeMissing",
    "SealInvalid",
    "SecurityError",
    "SessionRevoked",
    "SubjectNotFound",
    "TenancyError",
    "TenantDisabled",
    "TenantMismatch",
    "TenantNotFound",
    "TransformFailed",
    "VerifierFailed",
]


class SecurityError(Exception):
    """Root of the M11 error hierarchy."""

    code: str = "security.error"
    http_status: int = 500
    retryable_hint: bool = False

    def __init__(self, message: str, *, detail: Any = None) -> None:
        super().__init__(message)
        self.message = message
        self.detail = detail

    def to_dict(self) -> dict[str, Any]:
        out: dict[str, Any] = {
            "code": self.code,
            "message": self.message,
            "retryable": self.retryable_hint,
            "http_status": self.http_status,
        }
        if self.detail is not None:
            out["detail"] = self.detail
        return out

    def __repr__(self) -> str:  # pragma: no cover
        return f"{type(self).__name__}(code={self.code!r}, message={self.message!r})"


# ---------------------------------------------------------------------------
# AuthN
# ---------------------------------------------------------------------------


class AuthNError(SecurityError):
    code = "authn.error"
    http_status = 401


class MissingToken(AuthNError):
    code = "authn.missing_token"


class InvalidSignature(AuthNError):
    code = "authn.invalid_signature"


class ExpiredToken(AuthNError):
    code = "authn.expired_token"


class InvalidAudience(AuthNError):
    code = "authn.invalid_audience"


class InvalidIssuer(AuthNError):
    code = "authn.invalid_issuer"


class InvalidKid(AuthNError):
    code = "authn.invalid_kid"


class MfaRequired(AuthNError):
    code = "authn.mfa_required"
    http_status = 403


class SessionRevoked(AuthNError):
    code = "authn.session_revoked"
    http_status = 401


# ---------------------------------------------------------------------------
# AuthZ
# ---------------------------------------------------------------------------


class AuthZError(SecurityError):
    code = "authz.error"
    http_status = 403


class ScopeMissing(AuthZError):
    code = "authz.scope_missing"


class OwnershipViolation(AuthZError):
    code = "authz.ownership_violation"


class AbacConditionFailed(AuthZError):
    code = "authz.abac_failed"


class TenantMismatch(AuthZError):
    code = "authz.tenant_mismatch"


class ActionForbidden(AuthZError):
    code = "authz.action_forbidden"


# ---------------------------------------------------------------------------
# Tenancy
# ---------------------------------------------------------------------------


class TenancyError(SecurityError):
    code = "tenancy.error"
    http_status = 409


class TenantNotFound(TenancyError):
    code = "tenancy.not_found"
    http_status = 404


class TenantDisabled(TenancyError):
    code = "tenancy.disabled"
    http_status = 403


class QuotaExceeded(TenancyError):
    code = "tenancy.quota_exceeded"
    http_status = 429
    retryable_hint = True


# ---------------------------------------------------------------------------
# Crypto
# ---------------------------------------------------------------------------


class CryptoError(SecurityError):
    code = "crypto.error"
    http_status = 500


class KeyNotFound(CryptoError):
    code = "crypto.key_not_found"
    http_status = 404


class KeyExpired(CryptoError):
    code = "crypto.key_expired"


class SealInvalid(CryptoError):
    code = "crypto.seal_invalid"
    http_status = 409


class AlgorithmDeprecated(CryptoError):
    code = "crypto.algorithm_deprecated"
    http_status = 409


class RotationFailed(CryptoError):
    code = "crypto.rotation_failed"
    retryable_hint = True


# ---------------------------------------------------------------------------
# PII
# ---------------------------------------------------------------------------


class PiiError(SecurityError):
    code = "pii.error"
    http_status = 409


class FieldNotClassified(PiiError):
    code = "pii.field_not_classified"
    http_status = 500


class TransformFailed(PiiError):
    code = "pii.transform_failed"


class PolicyViolation(PiiError):
    code = "pii.policy_violation"
    http_status = 403


# ---------------------------------------------------------------------------
# Audit
# ---------------------------------------------------------------------------


class AuditError(SecurityError):
    code = "audit.error"
    http_status = 500


class ChainBroken(AuditError):
    code = "audit.chain_broken"


class VerifierFailed(AuditError):
    code = "audit.verifier_failed"


class ExportDenied(AuditError):
    code = "audit.export_denied"
    http_status = 403


# ---------------------------------------------------------------------------
# Consent
# ---------------------------------------------------------------------------


class ConsentError(SecurityError):
    code = "consent.error"
    http_status = 403


class ConsentMissing(ConsentError):
    code = "consent.missing"


class ConsentExpired(ConsentError):
    code = "consent.expired"


class ConsentWithdrawn(ConsentError):
    code = "consent.withdrawn"


class PurposeNotDeclared(ConsentError):
    code = "consent.purpose_not_declared"


# ---------------------------------------------------------------------------
# DSAR
# ---------------------------------------------------------------------------


class DsarError(SecurityError):
    code = "dsar.error"
    http_status = 409


class SubjectNotFound(DsarError):
    code = "dsar.subject_not_found"
    http_status = 404


class RequestAlreadyProcessed(DsarError):
    code = "dsar.already_processed"


class LegalHoldInForce(DsarError):
    code = "dsar.legal_hold"


# ---------------------------------------------------------------------------
# Lifecycle
# ---------------------------------------------------------------------------


class LifecycleError(SecurityError):
    code = "lifecycle.error"
    http_status = 500


class ErasureBlocked(LifecycleError):
    code = "lifecycle.erasure_blocked"
    http_status = 409


class RetentionConflict(LifecycleError):
    code = "lifecycle.retention_conflict"
    http_status = 409


# ---------------------------------------------------------------------------
# Incident
# ---------------------------------------------------------------------------


class IncidentError(SecurityError):
    code = "incident.error"
    http_status = 500


class PlaybookFailed(IncidentError):
    code = "incident.playbook_failed"


class NotificationBlocked(IncidentError):
    code = "incident.notification_blocked"
