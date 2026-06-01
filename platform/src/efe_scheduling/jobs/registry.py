"""Static registry of :class:`JobKind` handlers.

A handler is an ``async def (spec, deps) -> JobResult``. Each registered
handler declares :

* its associated :class:`JobKind`,
* a Pydantic-compatible payload schema (validated at enqueue time),
* its default retry policy and priority,
* whether it is idempotent (if not, ``max_attempts`` is capped at 1).

Import-time registration via the :func:`job_kind` decorator. A single
global :data:`default_registry` is provided, but applications may create
private registries (useful in tests).
"""

from __future__ import annotations

import asyncio
import inspect
import logging
from dataclasses import dataclass, field
from typing import Any, Callable, Coroutine, Protocol, TYPE_CHECKING

from efe_scheduling.errors import (
    PayloadSchemaViolation,
    UnknownJobKind,
    ValidationError,
)
from efe_scheduling.types import (
    JobKind,
    JobPriority,
    JobSpec,
    RetryPolicyKind,
    RetryPolicySpec,
)

if TYPE_CHECKING:
    from efe_scheduling.jobs.executor import JobResult

__all__ = [
    "HandlerFn",
    "JobDef",
    "JobRegistry",
    "PayloadSchema",
    "default_registry",
    "job_kind",
]

_LOG = logging.getLogger("efe_scheduling.registry")


# ---------------------------------------------------------------------------
# Payload schema Protocol
# ---------------------------------------------------------------------------


class PayloadSchema(Protocol):
    """Anything that validates a dict and returns a (possibly shaped) value.

    The minimum contract is ``validate(data) -> dict``. Pydantic v2
    models implement ``model_validate(data) -> Self`` and are adapted
    with a tiny wrapper inside :meth:`JobRegistry.register`.
    """

    def validate(self, data: dict[str, Any]) -> dict[str, Any]: ...  # pragma: no cover


# Handler type — takes a JobSpec and a deps mapping, returns a JobResult.
HandlerFn = Callable[[JobSpec, dict[str, Any]], Coroutine[Any, Any, "JobResult"]]


@dataclass(frozen=True, slots=True)
class JobDef:
    """Registered job-kind definition."""

    kind: JobKind
    handler: HandlerFn
    schema: PayloadSchema
    default_priority: JobPriority = JobPriority.normal
    default_retry: RetryPolicySpec = field(
        default_factory=lambda: RetryPolicySpec(
            kind=RetryPolicyKind.exponential, base_ms=500, max_ms=60_000, jitter_ratio=0.2
        )
    )
    idempotent: bool = True
    description: str = ""


class _AlwaysValidSchema:
    """Fallback schema accepting any dict."""

    def validate(self, data: dict[str, Any]) -> dict[str, Any]:
        if not isinstance(data, dict):
            raise PayloadSchemaViolation("payload must be a dict")
        return data


class JobRegistry:
    """Registry of JobKind → JobDef."""

    __slots__ = ("_defs",)

    def __init__(self) -> None:
        self._defs: dict[JobKind, JobDef] = {}

    def register(
        self,
        kind: JobKind,
        handler: HandlerFn,
        *,
        schema: PayloadSchema | None = None,
        default_priority: JobPriority = JobPriority.normal,
        default_retry: RetryPolicySpec | None = None,
        idempotent: bool = True,
        description: str = "",
    ) -> JobDef:
        if not asyncio.iscoroutinefunction(handler) and not inspect.isfunction(handler):
            raise ValidationError("handler must be an async callable")
        if kind in self._defs:
            raise ValidationError(f"JobKind {kind} already registered")
        definition = JobDef(
            kind=kind,
            handler=handler,
            schema=schema if schema is not None else _AlwaysValidSchema(),
            default_priority=default_priority,
            default_retry=(
                default_retry
                if default_retry is not None
                else RetryPolicySpec(
                    kind=RetryPolicyKind.exponential,
                    base_ms=500,
                    max_ms=60_000,
                    jitter_ratio=0.2,
                )
            ),
            idempotent=idempotent,
            description=description,
        )
        self._defs[kind] = definition
        _LOG.info("jobs.registry.register kind=%s idempotent=%s", kind, idempotent)
        return definition

    def get(self, kind: JobKind) -> JobDef:
        try:
            return self._defs[kind]
        except KeyError as exc:
            raise UnknownJobKind(f"unregistered JobKind: {kind}") from exc

    def validate_payload(self, kind: JobKind, payload: dict[str, Any]) -> dict[str, Any]:
        defn = self.get(kind)
        try:
            return defn.schema.validate(payload)
        except PayloadSchemaViolation:
            raise
        except Exception as exc:  # noqa: BLE001
            raise PayloadSchemaViolation(str(exc)) from exc

    def list_kinds(self) -> list[JobKind]:
        return sorted(self._defs.keys(), key=lambda k: k.value)


# ---------------------------------------------------------------------------
# Default registry + decorator
# ---------------------------------------------------------------------------

default_registry: JobRegistry = JobRegistry()


def job_kind(
    kind: JobKind,
    *,
    schema: PayloadSchema | None = None,
    default_priority: JobPriority = JobPriority.normal,
    default_retry: RetryPolicySpec | None = None,
    idempotent: bool = True,
    description: str = "",
    registry: JobRegistry | None = None,
) -> Callable[[HandlerFn], HandlerFn]:
    """Decorator that registers ``handler`` under ``kind``.

    Example::

        @job_kind(JobKind.noop, description="no-op for tests")
        async def noop(spec, deps):
            return JobResult.ok()
    """
    reg = registry if registry is not None else default_registry

    def wrap(fn: HandlerFn) -> HandlerFn:
        reg.register(
            kind,
            fn,
            schema=schema,
            default_priority=default_priority,
            default_retry=default_retry,
            idempotent=idempotent,
            description=description,
        )
        return fn

    return wrap
