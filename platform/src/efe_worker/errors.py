"""Exception hierarchy for :mod:`efe_worker`.

Design principles (mirrors ``efe_ft`` / ``efe_orch`` for symmetry):

* A single root :class:`WorkerError` so callers can trap any worker
  failure with one ``except`` clause — important at the
  :func:`efe_worker.__main__` top-level boundary where the last-resort
  handler catches *one* base type before reporting to M03.
* Every subclass carries an immutable :attr:`code` — the canonical
  wire identifier that M03 receives in the ``ReportError`` RPC. Codes
  are snake_case, stable across versions, and documented in the M04
  architecture doc (§16 "Gestion des erreurs").
* :attr:`retryable_hint` is *informational* — the authoritative retry
  decision lives on the orchestrator side. We surface our best guess
  so logs and metrics can be tagged consistently.
* :attr:`detail` is an arbitrary JSON-serialisable dict; it's attached
  to structured logs and the ``ReportError.error_message`` field.
"""

from __future__ import annotations

from typing import Any


class WorkerError(Exception):
    """Root of the efe-worker error hierarchy."""

    code: str = "worker_error"
    retryable_hint: bool = False

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
# Configuration / setup
# ---------------------------------------------------------------------------


class ConfigError(WorkerError):
    code = "config_error"


class MissingDependency(WorkerError):
    """An optional dependency (torch, peft, boto3) is not installed."""

    code = "missing_dependency"


# ---------------------------------------------------------------------------
# Transport / orchestrator contract
# ---------------------------------------------------------------------------


class TransportError(WorkerError):
    code = "transport_error"
    retryable_hint = True


class HeartbeatTimeout(TransportError):
    code = "heartbeat_timeout"
    retryable_hint = True


class OrchestratorUnreachable(TransportError):
    code = "orchestrator_unreachable"
    retryable_hint = True


# ---------------------------------------------------------------------------
# Storage
# ---------------------------------------------------------------------------


class StorageError(WorkerError):
    code = "storage_error"
    retryable_hint = True


class S3KeyNotFound(StorageError):
    code = "s3_key_not_found"
    retryable_hint = False


class S3Checksum(StorageError):
    """sha256 of downloaded blob does not match the manifest value."""

    code = "s3_checksum_mismatch"
    retryable_hint = True


# ---------------------------------------------------------------------------
# Data
# ---------------------------------------------------------------------------


class DataError(WorkerError):
    code = "data_error"


class SnapshotMissing(DataError):
    code = "snapshot_missing"


class CorruptDataset(DataError):
    code = "corrupt_dataset"


class LabelImbalance(DataError):
    code = "label_imbalance"


class TokenizationError(DataError):
    code = "tokenization_error"


# ---------------------------------------------------------------------------
# Training
# ---------------------------------------------------------------------------


class TrainingError(WorkerError):
    code = "training_error"


class TrainingDiverged(TrainingError):
    """Loss became NaN / Inf — run is unrecoverable."""

    code = "training_diverged"


class InvalidHyperparameters(TrainingError):
    code = "invalid_hyperparameters"


class UnknownRecipe(TrainingError):
    """The configured ``method_name`` is not in the registry."""

    code = "unknown_recipe"


class InvalidBaseModel(TrainingError):
    code = "invalid_base_model"


# ---------------------------------------------------------------------------
# Hardware
# ---------------------------------------------------------------------------


class CudaOOM(WorkerError):
    code = "cuda_oom"
    retryable_hint = True


class WorkerOOM(WorkerError):
    code = "worker_oom"
    retryable_hint = True


class ActiveDeadlineExceeded(WorkerError):
    code = "active_deadline"
    retryable_hint = True


class UserDeadlineExceeded(WorkerError):
    code = "user_deadline"
    retryable_hint = True


# ---------------------------------------------------------------------------
# Result / artifact upload
# ---------------------------------------------------------------------------


class ResultUploadFailed(WorkerError):
    code = "result_upload_failed"
    retryable_hint = False


# ---------------------------------------------------------------------------
# Lifecycle
# ---------------------------------------------------------------------------


class SigtermGraceful(WorkerError):
    """Pod received SIGTERM and shut down cleanly."""

    code = "sigterm_graceful"
    retryable_hint = True


__all__ = [
    "ActiveDeadlineExceeded",
    "ConfigError",
    "CorruptDataset",
    "CudaOOM",
    "DataError",
    "HeartbeatTimeout",
    "InvalidBaseModel",
    "InvalidHyperparameters",
    "LabelImbalance",
    "MissingDependency",
    "OrchestratorUnreachable",
    "ResultUploadFailed",
    "S3Checksum",
    "S3KeyNotFound",
    "SigtermGraceful",
    "SnapshotMissing",
    "StorageError",
    "TokenizationError",
    "TrainingDiverged",
    "TrainingError",
    "TransportError",
    "UnknownRecipe",
    "UserDeadlineExceeded",
    "WorkerError",
    "WorkerOOM",
]
