"""Exception hierarchy for :mod:`efe_ft`.

Design:

* Single root :class:`FtError` — any caller can trap platform-level
  failures with one ``except`` clause.
* Subclasses carry an immutable :attr:`code` for machine-readable
  routing in audit logs, metrics, and HTTP problem-detail bodies.
* Retryable failures expose :attr:`retryable` so client-side loops can
  back off without hard-coding a list.
"""

from __future__ import annotations

from typing import Any


class FtError(Exception):
    """Root of the efe-ft error hierarchy."""

    code: str = "ft_error"
    retryable: bool = False
    http_status: int = 500

    def __init__(self, message: str, *, detail: Any = None) -> None:
        super().__init__(message)
        self.message = message
        self.detail = detail

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "code": self.code,
            "message": self.message,
            "retryable": self.retryable,
        }
        if self.detail is not None:
            payload["detail"] = self.detail
        return payload

    def __repr__(self) -> str:
        return f"{type(self).__name__}(code={self.code!r}, message={self.message!r})"


# ---------------------------------------------------------------------------
#  Configuration / setup errors — not retryable, not user-facing
# ---------------------------------------------------------------------------


class ConfigError(FtError):
    code = "config_error"
    http_status = 500


# ---------------------------------------------------------------------------
#  Ingestion errors
# ---------------------------------------------------------------------------


class IngestError(FtError):
    code = "ingest_error"
    http_status = 400


class SchemaInvalid(IngestError):
    code = "schema_invalid"
    http_status = 400


class PiiBlocked(IngestError):
    code = "pii_blocked"
    http_status = 400


class Duplicate(IngestError):
    """Not strictly an error — the caller receives an *absorbed* response."""

    code = "duplicate"
    http_status = 200


class BalanceSkew(IngestError):
    code = "balance_skew"
    http_status = 409


class QuotaExceeded(IngestError):
    code = "quota_exceeded"
    http_status = 429
    retryable = False


# ---------------------------------------------------------------------------
#  Storage / DB errors
# ---------------------------------------------------------------------------


class StorageError(FtError):
    code = "storage_error"
    http_status = 503
    retryable = True


class RlsViolation(FtError):
    code = "rls_violation"
    http_status = 403


# ---------------------------------------------------------------------------
#  Audit
# ---------------------------------------------------------------------------


class AuditChainBroken(FtError):
    code = "audit_chain_broken"
    http_status = 500


# ---------------------------------------------------------------------------
#  Labeling errors
# ---------------------------------------------------------------------------


class LabelingError(FtError):
    code = "labeling_error"
    http_status = 400


class UnknownLabel(LabelingError):
    code = "unknown_label"
    http_status = 422


class SessionExpired(LabelingError):
    code = "session_expired"
    http_status = 410


class SampleNotAssigned(LabelingError):
    code = "sample_not_assigned"
    http_status = 409


class KappaTooLow(LabelingError):
    """Not raised — reported; used as a sentinel for monitoring code."""

    code = "kappa_too_low"


# ---------------------------------------------------------------------------
#  Active learning / embedder
# ---------------------------------------------------------------------------


class ActiveLearningError(FtError):
    code = "active_learning_error"


class EmbedderError(FtError):
    code = "embedder_error"
    retryable = True


# ---------------------------------------------------------------------------
#  Snapshot
# ---------------------------------------------------------------------------


class SnapshotError(FtError):
    code = "snapshot_error"


class SnapshotImmutable(SnapshotError):
    code = "snapshot_immutable"
    http_status = 409


__all__ = [
    "ActiveLearningError",
    "AuditChainBroken",
    "BalanceSkew",
    "ConfigError",
    "Duplicate",
    "EmbedderError",
    "FtError",
    "IngestError",
    "KappaTooLow",
    "LabelingError",
    "PiiBlocked",
    "QuotaExceeded",
    "RlsViolation",
    "SampleNotAssigned",
    "SchemaInvalid",
    "SessionExpired",
    "SnapshotError",
    "SnapshotImmutable",
    "StorageError",
    "UnknownLabel",
]
