"""Exposure recording — invariant **I5** enforcement.

The :class:`ExposureRecorder` turns an :class:`AssignmentDecision`
plus context into an HMAC-signed :class:`Exposure` and pushes it into
a :class:`ExposureSink`. Sinks can be:

* :class:`InMemoryExposureSink` — unit tests and single-process runs.
* A NATS publisher (see :mod:`efe_rollout.nats`) — production.
* A DB sink via :class:`~efe_rollout.db.ExposureRow` — audit queries.

The HMAC uses the rollout exposure domain tag so a key legitimately
reused elsewhere (NATS envelope, pre-reg) cannot be replayed across
them.
"""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass
from typing import Protocol

from efe_rollout.assignment.assigner import AssignmentDecision
from efe_rollout.clock import Clock, SystemClock
from efe_rollout.ids import new_uuid
from efe_rollout.records import Exposure


class ExposureSink(Protocol):
    async def write(
        self, exposure: Exposure,
    ) -> None:  # pragma: no cover
        ...


class InMemoryExposureSink:
    """Append-only sink for tests.

    Not thread-safe — but within a single asyncio event loop it's
    enough: callers don't await in between ``write`` and ``all()``.
    """

    __slots__ = ("_rows",)

    def __init__(self) -> None:
        self._rows: list[Exposure] = []

    async def write(self, exposure: Exposure) -> None:
        self._rows.append(exposure)

    def all(self) -> tuple[Exposure, ...]:
        return tuple(self._rows)

    def for_experiment(
        self, experiment_id: str,
    ) -> tuple[Exposure, ...]:
        return tuple(
            r for r in self._rows if r.experiment_id == experiment_id
        )

    def for_user(
        self, experiment_id: str, user_id: str,
    ) -> tuple[Exposure, ...]:
        return tuple(
            r for r in self._rows
            if r.experiment_id == experiment_id and r.user_id == user_id
        )


@dataclass(frozen=True, slots=True)
class ExposureContext:
    """Everything the recorder needs besides the decision."""

    tenant_id: str
    experiment_id: str
    user_id: str
    layer: str
    request_id: str | None = None
    segment_hash: bytes | None = None


class ExposureRecorder:
    """Produce HMAC-signed :class:`Exposure` rows."""

    __slots__ = ("_clock", "_hmac_key", "_sinks")

    def __init__(
        self,
        *,
        sinks: Iterable[ExposureSink],
        hmac_key: bytes,
        clock: Clock | None = None,
    ) -> None:
        self._sinks = tuple(sinks)
        if len(hmac_key) < 16:
            raise ValueError(
                f"exposure HMAC key too short ({len(hmac_key)} bytes, need 16+)",
            )
        self._hmac_key = bytes(hmac_key)
        self._clock = clock or SystemClock()

    async def record(
        self,
        ctx: ExposureContext,
        decision: AssignmentDecision,
    ) -> Exposure:
        exposure = Exposure(
            exposure_id=new_uuid(),
            experiment_id=ctx.experiment_id,
            tenant_id=ctx.tenant_id,
            user_id=ctx.user_id,
            assigned_at=self._clock.now(),
            layer=ctx.layer,
            forced=decision.forced,
            holdout=decision.holdout,
            variant_id=(
                decision.variant.variant_id if decision.variant else None
            ),
            request_id=ctx.request_id,
            segment_hash=ctx.segment_hash,
        ).sign(self._hmac_key)
        for sink in self._sinks:
            await sink.write(exposure)
        return exposure


__all__ = [
    "ExposureContext",
    "ExposureRecorder",
    "ExposureSink",
    "InMemoryExposureSink",
]
