"""Exception hierarchy for :mod:`efe_orch`.

Design principles (mirrors ``efe_ft`` for operational symmetry):

* A single root :class:`OrchError` so callers can trap any orchestrator
  failure with one ``except`` clause — important at gRPC & HTTP layer
  boundaries where the mapper catches *one* base type.
* Every subclass carries an immutable :attr:`code` string for
  machine-readable routing (audit logs, metrics labels, problem-details
  bodies). Codes are snake_case and never reused across subclasses.
* :attr:`retryable` is set on the subclass so client-side retry loops
  do not need to maintain their own allow-list of error types.
* :attr:`http_status` pins the default HTTP mapping; the gRPC mapper
  translates via a small table in :mod:`efe_orch.api`.

This module has **no** dependencies on the rest of the package — it is
safe to import from anywhere without creating cycles.
"""

from __future__ import annotations

from typing import Any


class OrchError(Exception):
    """Root of the efe-orch error hierarchy."""

    code: str = "orch_error"
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
#  Configuration / setup errors
# ---------------------------------------------------------------------------


class ConfigError(OrchError):
    code = "config_error"
    http_status = 500


# ---------------------------------------------------------------------------
#  Submission / input validation
# ---------------------------------------------------------------------------


class ValidationError(OrchError):
    code = "validation_error"
    http_status = 400


class RunNotFound(OrchError):
    code = "run_not_found"
    http_status = 404


class TenantMismatch(OrchError):
    code = "tenant_mismatch"
    http_status = 403


# ---------------------------------------------------------------------------
#  Scheduling & quota
# ---------------------------------------------------------------------------


class NoSuitablePool(OrchError):
    code = "no_suitable_pool"
    http_status = 409


class QuotaExceeded(OrchError):
    code = "quota_exceeded"
    http_status = 429


class PoolSaturated(OrchError):
    """Transient — all workers in the chosen pool are busy."""

    code = "pool_saturated"
    http_status = 503
    retryable = True


# ---------------------------------------------------------------------------
#  State machine
# ---------------------------------------------------------------------------


class StateMachineError(OrchError):
    code = "fsm_error"
    http_status = 409


class InvalidTransition(StateMachineError):
    code = "invalid_transition"
    http_status = 409


class StateRaceDetected(StateMachineError):
    """Two writers tried to mutate the same run concurrently.

    The loser retries after re-reading the latest state; the winner
    wins the advisory lock.
    """

    code = "state_race"
    http_status = 409
    retryable = True


class RunTerminated(StateMachineError):
    """Attempted to mutate a run that is already in a terminal state."""

    code = "run_terminated"
    http_status = 409


# ---------------------------------------------------------------------------
#  Dispatcher
# ---------------------------------------------------------------------------


class DispatcherError(OrchError):
    code = "dispatcher_error"
    http_status = 503
    retryable = True


class WorkerJobRejected(DispatcherError):
    """K8s API refused the Job manifest (admission webhook, etc.)."""

    code = "worker_job_rejected"
    http_status = 500
    retryable = False


# ---------------------------------------------------------------------------
#  Worker contract
# ---------------------------------------------------------------------------


class WorkerProtocolError(OrchError):
    code = "worker_protocol_error"
    http_status = 400


class UnknownWorker(OrchError):
    code = "unknown_worker"
    http_status = 404


class StaleHeartbeat(OrchError):
    code = "stale_heartbeat"
    http_status = 409


# ---------------------------------------------------------------------------
#  Cancellation / authz
# ---------------------------------------------------------------------------


class CannotCancel(OrchError):
    code = "cannot_cancel"
    http_status = 409


# ---------------------------------------------------------------------------
#  Persistence
# ---------------------------------------------------------------------------


class StorageError(OrchError):
    code = "storage_error"
    http_status = 503
    retryable = True


__all__ = [
    "CannotCancel",
    "ConfigError",
    "DispatcherError",
    "InvalidTransition",
    "NoSuitablePool",
    "OrchError",
    "PoolSaturated",
    "QuotaExceeded",
    "RunNotFound",
    "RunTerminated",
    "StaleHeartbeat",
    "StateMachineError",
    "StateRaceDetected",
    "StorageError",
    "TenantMismatch",
    "UnknownWorker",
    "ValidationError",
    "WorkerJobRejected",
    "WorkerProtocolError",
]
