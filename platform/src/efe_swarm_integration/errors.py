"""Exception taxonomy for :mod:`efe_swarm_integration` — M12.

Mirrors §20 of the M12 architecture. Every class carries a stable
``code`` and a ``http_status``. All descend from
:class:`SwarmIntegrationError` so one ``except`` traps the whole module.
"""

from __future__ import annotations

from typing import Any

__all__ = [
    "AlignmentBreach",
    "ArtefactDigestMismatch",
    "ArtefactError",
    "ArtefactSyncFailed",
    "AuthNError",
    "BackpressureRejected",
    "CatalogCompilationFailed",
    "CatalogError",
    "CatalogNotFound",
    "ChannelUnavailable",
    "CircuitOpen",
    "ConeAlreadyExists",
    "ConeRefNotBound",
    "ContractError",
    "ContractSnapshotMismatch",
    "FlowControlError",
    "InvalidConeOp",
    "MutualTlsFailed",
    "OperationCancelled",
    "OptimisticLockConflict",
    "PiiDetectedInPrompt",
    "QuotaExceeded",
    "SchemaViolation",
    "SchemaVersionMismatch",
    "SecurityError",
    "StreamAborted",
    "StreamingError",
    "SwarmDataLoss",
    "SwarmDivergence",
    "SwarmInternalError",
    "SwarmIntegrationError",
    "SwarmModeCollapse",
    "SwarmTimeout",
    "SwarmUnavailable",
    "SwarmUnknownError",
    "TenantMismatch",
    "TimeoutExceeded",
    "TransportError",
    "UnauthorisedM12",
    "UncertainAlignment",
    "UnexpectedChunk",
    "UnsupportedRpc",
    "VerdictError",
    "HbmBudgetExceeded",
]


class SwarmIntegrationError(Exception):
    """Root of the M12 error hierarchy."""

    code: str = "swarm.integration_error"
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
# Transport
# ---------------------------------------------------------------------------


class TransportError(SwarmIntegrationError):
    code = "swarm.transport"
    http_status = 503
    retryable_hint = True


class ChannelUnavailable(TransportError):
    code = "swarm.transport.unavailable"


class MutualTlsFailed(TransportError):
    code = "swarm.transport.mtls_failed"
    http_status = 500
    retryable_hint = False


class TimeoutExceeded(TransportError):
    code = "swarm.transport.timeout"
    http_status = 504


class OperationCancelled(TransportError):
    code = "swarm.transport.cancelled"
    http_status = 499
    retryable_hint = False


# ---------------------------------------------------------------------------
# Contract
# ---------------------------------------------------------------------------


class ContractError(SwarmIntegrationError):
    code = "swarm.contract"
    http_status = 400


class SchemaViolation(ContractError):
    code = "swarm.contract.schema_violation"


class SchemaVersionMismatch(ContractError):
    code = "swarm.contract.schema_version_mismatch"


class ContractSnapshotMismatch(ContractError):
    code = "swarm.contract.snapshot_mismatch"
    http_status = 500


# ---------------------------------------------------------------------------
# Catalog
# ---------------------------------------------------------------------------


class CatalogError(SwarmIntegrationError):
    code = "swarm.catalog"
    http_status = 404


class CatalogNotFound(CatalogError):
    code = "swarm.catalog.not_found"


class CatalogCompilationFailed(CatalogError):
    code = "swarm.catalog.compilation_failed"
    http_status = 400


class ConeRefNotBound(CatalogError):
    code = "swarm.catalog.cone_not_bound"


class ConeAlreadyExists(CatalogError):
    code = "swarm.catalog.cone_exists"
    http_status = 409


class InvalidConeOp(CatalogError):
    code = "swarm.catalog.invalid_op"
    http_status = 409


# ---------------------------------------------------------------------------
# Artefact
# ---------------------------------------------------------------------------


class ArtefactError(SwarmIntegrationError):
    code = "swarm.artefact"
    http_status = 500


class ArtefactDigestMismatch(ArtefactError):
    code = "swarm.artefact.digest_mismatch"


class ArtefactSyncFailed(ArtefactError):
    code = "swarm.artefact.sync_failed"
    retryable_hint = True


# ---------------------------------------------------------------------------
# Verdict
# ---------------------------------------------------------------------------


class VerdictError(SwarmIntegrationError):
    code = "swarm.verdict"
    http_status = 422


class SwarmDivergence(VerdictError):
    code = "swarm.verdict.divergence"


class SwarmModeCollapse(VerdictError):
    code = "swarm.verdict.mode_collapse"


class AlignmentBreach(VerdictError):
    code = "swarm.verdict.alignment_breach"


class UncertainAlignment(VerdictError):
    code = "swarm.verdict.uncertain_alignment"


class SwarmUnknownError(SwarmIntegrationError):
    code = "swarm.internal.unknown"
    http_status = 500


class SwarmInternalError(SwarmIntegrationError):
    code = "swarm.internal.server_error"
    http_status = 500


class SwarmDataLoss(SwarmIntegrationError):
    code = "swarm.internal.data_loss"
    http_status = 500


class SwarmUnavailable(TransportError):
    code = "swarm.transport.service_unavailable"


class SwarmTimeout(TransportError):
    code = "swarm.transport.deadline_exceeded"
    http_status = 504


# ---------------------------------------------------------------------------
# Flow control
# ---------------------------------------------------------------------------


class FlowControlError(SwarmIntegrationError):
    code = "swarm.flow_control"
    http_status = 503
    retryable_hint = True


class BackpressureRejected(FlowControlError):
    code = "swarm.flow_control.backpressure"


class CircuitOpen(FlowControlError):
    code = "swarm.flow_control.circuit_open"


class QuotaExceeded(FlowControlError):
    code = "swarm.flow_control.quota_exceeded"
    http_status = 429


class HbmBudgetExceeded(SwarmIntegrationError):
    code = "swarm.budget.hbm_exceeded"
    http_status = 413  # Payload Too Large ≈ budget overrun


# ---------------------------------------------------------------------------
# Security
# ---------------------------------------------------------------------------


class SecurityError(SwarmIntegrationError):
    code = "swarm.security"
    http_status = 403


class PiiDetectedInPrompt(SecurityError):
    code = "swarm.security.pii_in_prompt"
    http_status = 400


class TenantMismatch(SecurityError):
    code = "swarm.security.tenant_mismatch"


class UnauthorisedM12(SecurityError):
    code = "swarm.security.unauthorised"
    http_status = 403


class AuthNError(SecurityError):
    code = "swarm.security.authn_failed"
    http_status = 401


# ---------------------------------------------------------------------------
# Streaming
# ---------------------------------------------------------------------------


class StreamingError(SwarmIntegrationError):
    code = "swarm.streaming"
    http_status = 500


class StreamAborted(StreamingError):
    code = "swarm.streaming.aborted"
    retryable_hint = True


class UnexpectedChunk(StreamingError):
    code = "swarm.streaming.unexpected_chunk"


# ---------------------------------------------------------------------------
# Concurrency control (RPC echo)
# ---------------------------------------------------------------------------


class OptimisticLockConflict(SwarmIntegrationError):
    code = "swarm.concurrency.optimistic_lock"
    http_status = 409
    retryable_hint = True


class UnsupportedRpc(SwarmIntegrationError):
    code = "swarm.rpc.unsupported"
    http_status = 501
