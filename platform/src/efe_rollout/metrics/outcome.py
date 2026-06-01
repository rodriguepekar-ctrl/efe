"""Outcome recording — metric observations tied to a (user, experiment).

The :class:`OutcomeRecorder` is the twin of :class:`ExposureRecorder`
but for metric observations (clicks, latency, thumbs-up, cost, …).
Unlike exposures, outcomes do *not* carry an HMAC — they are ingested
in bulk and integrity is checked at the NATS envelope level.

Outcomes may arrive :

* synchronously as a REST ``POST /v1/rollout/outcomes`` call (buffered
  then flushed every second), or
* asynchronously as an ``efe.inference.completed`` NATS event consumed
  by :mod:`efe_rollout.nats`.

ITT vs PP semantics :

* When :attr:`strict_itt` is True, we refuse outcomes for
  ``(user, experiment)`` pairs with *no* prior exposure —
  :class:`OutcomeWithoutExposure` is raised.
* When it's False, outcomes still land in the sink and the analysis
  pipeline decides whether to drop them at compute-time.
"""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass
from typing import Protocol

from efe_rollout.clock import Clock, SystemClock
from efe_rollout.errors import OutcomeWithoutExposure
from efe_rollout.ids import new_uuid
from efe_rollout.records import Outcome


class OutcomeSink(Protocol):
    async def write(
        self, outcome: Outcome,
    ) -> None:  # pragma: no cover
        ...


class InMemoryOutcomeSink:
    __slots__ = ("_rows",)

    def __init__(self) -> None:
        self._rows: list[Outcome] = []

    async def write(self, outcome: Outcome) -> None:
        self._rows.append(outcome)

    def all(self) -> tuple[Outcome, ...]:
        return tuple(self._rows)

    def for_experiment(
        self, experiment_id: str,
    ) -> tuple[Outcome, ...]:
        return tuple(
            r for r in self._rows if r.experiment_id == experiment_id
        )


class ExposureIndex(Protocol):
    """Read-only view over existing exposures used for ITT enforcement."""

    async def exposed(
        self, *, experiment_id: str, user_id: str,
    ) -> bool:  # pragma: no cover
        ...


class _NullExposureIndex:
    async def exposed(
        self, *, experiment_id: str, user_id: str,
    ) -> bool:
        return True


@dataclass(frozen=True, slots=True)
class OutcomeContext:
    tenant_id: str
    experiment_id: str
    user_id: str
    variant_id: str | None
    metric_name: str
    metric_value: float
    request_id: str | None = None
    covariate: dict[str, object] | None = None


class OutcomeRecorder:
    """Turn a :class:`OutcomeContext` into a persisted :class:`Outcome`."""

    __slots__ = ("_clock", "_index", "_sinks", "_strict_itt")

    def __init__(
        self,
        *,
        sinks: Iterable[OutcomeSink],
        clock: Clock | None = None,
        exposure_index: ExposureIndex | None = None,
        strict_itt: bool = False,
    ) -> None:
        self._sinks = tuple(sinks)
        self._clock = clock or SystemClock()
        self._index: ExposureIndex = exposure_index or _NullExposureIndex()
        self._strict_itt = bool(strict_itt)

    async def record(self, ctx: OutcomeContext) -> Outcome:
        if self._strict_itt:
            ok = await self._index.exposed(
                experiment_id=ctx.experiment_id, user_id=ctx.user_id,
            )
            if not ok:
                raise OutcomeWithoutExposure(
                    f"no exposure for (experiment={ctx.experiment_id!r}, "
                    f"user={ctx.user_id!r}) — strict ITT refuses outcome",
                )
        outcome = Outcome(
            outcome_id=new_uuid(),
            experiment_id=ctx.experiment_id,
            tenant_id=ctx.tenant_id,
            user_id=ctx.user_id,
            variant_id=ctx.variant_id,
            metric_name=ctx.metric_name,
            metric_value=float(ctx.metric_value),
            observed_at=self._clock.now(),
            request_id=ctx.request_id,
            covariate=dict(ctx.covariate or {}),
        )
        for sink in self._sinks:
            await sink.write(outcome)
        return outcome


__all__ = [
    "ExposureIndex",
    "InMemoryOutcomeSink",
    "OutcomeContext",
    "OutcomeRecorder",
    "OutcomeSink",
]
