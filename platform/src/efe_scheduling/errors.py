"""Exception taxonomy for :mod:`efe_scheduling`.

All exceptions descend from :class:`SchedulingError` so one ``except``
clause traps the whole module. Every class carries :

* a stable ``code`` (audit logs + metric labels),
* a default ``http_status`` (REST mapping),
* a ``retryable_hint`` (client-side heuristic).

The tree mirrors §17 of ``M10_SCHEDULING_ARCHITECTURE.md``.
"""

from __future__ import annotations

from typing import Any

__all__ = [
    "ConfigError",
    "DebouncerCorrupt",
    "DuplicateFireKey",
    "HandlerCrashed",
    "IdempotencyReplay",
    "InvalidCronExpression",
    "InvalidRRule",
    "InvalidStateTransition",
    "InvalidTriggerSpec",
    "JobBudgetExhausted",
    "JobError",
    "JobNotFound",
    "JobSealInvalid",
    "JobTerminal",
    "LeaseHeartbeatFailed",
    "LeaseLost",
    "NotFound",
    "PayloadSchemaViolation",
    "PolicyNotFound",
    "ScheduleDisabled",
    "ScheduleError",
    "ScheduleNotFound",
    "SchedulerError",
    "SchedulingError",
    "ScopeMissing",
    "SecurityError",
    "StateVersionConflict",
    "StorageError",
    "TenantMismatch",
    "TimezoneNotFound",
    "TriggerError",
    "TriggerNotFound",
    "TriggerSuppressed",
    "UnknownJobKind",
    "ValidationError",
    "WorkerError",
]


class SchedulingError(Exception):
    """Root of the M10 error hierarchy."""

    code: str = "scheduling_error"
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
# Configuration / validation / NotFound
# ---------------------------------------------------------------------------


class ConfigError(SchedulingError):
    code = "config_error"
    http_status = 500


class ValidationError(SchedulingError):
    code = "validation_error"
    http_status = 400


class NotFound(SchedulingError):
    code = "not_found"
    http_status = 404


class ScheduleNotFound(NotFound):
    code = "schedule_not_found"


class JobNotFound(NotFound):
    code = "job_not_found"


class TriggerNotFound(NotFound):
    code = "trigger_not_found"


class PolicyNotFound(NotFound):
    code = "policy_not_found"


# ---------------------------------------------------------------------------
# Schedule errors
# ---------------------------------------------------------------------------


class ScheduleError(SchedulingError):
    code = "schedule_error"
    http_status = 400


class InvalidCronExpression(ScheduleError):
    code = "invalid_cron_expression"


class InvalidRRule(ScheduleError):
    code = "invalid_rrule"


class TimezoneNotFound(ScheduleError):
    code = "timezone_not_found"


class ScheduleDisabled(ScheduleError):
    code = "schedule_disabled"
    http_status = 409


# ---------------------------------------------------------------------------
# Trigger errors
# ---------------------------------------------------------------------------


class TriggerError(SchedulingError):
    code = "trigger_error"
    http_status = 400


class InvalidTriggerSpec(TriggerError):
    code = "invalid_trigger_spec"


class DebouncerCorrupt(TriggerError):
    code = "debouncer_corrupt"
    http_status = 500


class TriggerSuppressed(TriggerError):
    """Raised (non-fatal) when the debouncer suppresses a would-be fire."""

    code = "trigger_suppressed"
    http_status = 202  # Accepted but not acted upon.


# ---------------------------------------------------------------------------
# Job / worker errors
# ---------------------------------------------------------------------------


class JobError(SchedulingError):
    code = "job_error"
    http_status = 409


class DuplicateFireKey(JobError):
    code = "duplicate_fire_key"
    http_status = 409


class UnknownJobKind(JobError):
    code = "unknown_job_kind"
    http_status = 400


class PayloadSchemaViolation(JobError):
    code = "payload_schema_violation"
    http_status = 400


class JobSealInvalid(JobError):
    """Persisted job's HMAC seal does not match its payload."""

    code = "job_seal_invalid"
    http_status = 500


class JobTerminal(JobError):
    """Attempt to mutate a job that is already in a terminal state."""

    code = "job_terminal"


class JobBudgetExhausted(JobError):
    code = "job_budget_exhausted"


class WorkerError(SchedulingError):
    code = "worker_error"
    http_status = 500


class LeaseLost(WorkerError):
    code = "lease_lost"
    retryable_hint = True


class LeaseHeartbeatFailed(WorkerError):
    code = "lease_heartbeat_failed"
    retryable_hint = True


class HandlerCrashed(WorkerError):
    code = "handler_crashed"


# ---------------------------------------------------------------------------
# FSM & storage
# ---------------------------------------------------------------------------


class InvalidStateTransition(SchedulingError):
    code = "invalid_state_transition"
    http_status = 409


class StateVersionConflict(SchedulingError):
    """Optimistic-lock (CAS) failure on ``state_version``."""

    code = "state_version_conflict"
    http_status = 409
    retryable_hint = True


class StorageError(SchedulingError):
    code = "storage_error"
    http_status = 500
    retryable_hint = True


# ---------------------------------------------------------------------------
# Security
# ---------------------------------------------------------------------------


class SecurityError(SchedulingError):
    code = "security_error"
    http_status = 403


class TenantMismatch(SecurityError):
    code = "tenant_mismatch"


class ScopeMissing(SecurityError):
    code = "scope_missing"
    http_status = 403


class IdempotencyReplay(SecurityError):
    """Same idempotency key seen with a different payload."""

    code = "idempotency_replay"
    http_status = 409


# ---------------------------------------------------------------------------
# Generic scheduler error (reserved for leader/catch-up/etc.)
# ---------------------------------------------------------------------------


class SchedulerError(SchedulingError):
    code = "scheduler_error"
    http_status = 500
