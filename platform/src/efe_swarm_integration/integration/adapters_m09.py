"""Concrete M09 → M12 adapter.

M09 (:mod:`efe_drift`) is the ground truth for drift detection but its
:class:`efe_drift.detection.DriftResult` is shaped for its own
alerting / storage model. M10 (:mod:`efe_scheduling`) consumes a
slimmer :class:`efe_scheduling.triggers.DriftEvent` ; M12 needs to know
when a critical drift happens so it can :

1. evolve the cone bound to that feature (via :class:`AdminClient`),
2. audit the correlation via M11,
3. (optionally) trigger an artefact re-sync.

:class:`M09DriftAdapter` is the bridge that converts the DriftResult
into a typed ``DriftEvent`` and then dispatches to the M12 reaction
policy.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import Any, Awaitable, Callable

from efe_drift.detection import DriftResult
from efe_scheduling.triggers import DriftEvent, DriftSeverity

from efe_swarm_integration.admin.client import AdminClient
from efe_swarm_integration.admin.cone_ops import update_radius_op
from efe_swarm_integration.catalog.bridge import CatalogBridge
from efe_swarm_integration.errors import ConeRefNotBound
from efe_swarm_integration.types import ConeRef

__all__ = [
    "DriftReactionOutcome",
    "M09DriftAdapter",
    "drift_result_to_event",
]


# ---------------------------------------------------------------------------
# DriftResult → DriftEvent
# ---------------------------------------------------------------------------


def _normalise_severity(sev: str) -> DriftSeverity:
    mapping = {
        "none": DriftSeverity.none,
        "warning": DriftSeverity.warning,
        "major": DriftSeverity.major,
        "critical": DriftSeverity.critical,
    }
    return mapping.get(str(sev).lower(), DriftSeverity.none)


def drift_result_to_event(r: DriftResult) -> DriftEvent:
    """Convert a canonical :class:`efe_drift.detection.DriftResult` into
    a :class:`efe_scheduling.triggers.DriftEvent`.

    Preserves all fields M12 relies on : tenant, task, feature name,
    method, score, severity, windows.
    """
    return DriftEvent(
        result_id=r.evaluation_id,
        tenant_id=r.tenant_id,
        task_id=r.task_id,
        feature_name=r.feature_name,
        method=r.method,
        score=float(r.score),
        severity=_normalise_severity(r.severity),
        observed_at=_ensure_tz(r.computed_at),
        window_start=_ensure_tz(r.window_start),
        window_end=_ensure_tz(r.window_end),
    )


def _ensure_tz(dt: datetime) -> datetime:
    if dt.tzinfo is None:
        return dt.replace(tzinfo=timezone.utc)
    return dt


# ---------------------------------------------------------------------------
# Reaction policy
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class DriftReactionOutcome:
    event: DriftEvent
    action: str  # "cone_relaxed" | "noop" | "error"
    new_cone_key: str | None = None
    error: str | None = None


AuditEmitter = Callable[[DriftEvent, DriftReactionOutcome], Awaitable[None]]


@dataclass(slots=True)
class M09DriftAdapter:
    """Glue M09 → M12.

    Given a :class:`DriftResult` (M09) on a feature bound to a cone,
    decide whether to relax the cone radius (reduces false uncertain
    verdicts while a retrain is in flight) or to no-op.

    The adapter is **pure-policy** : it never mutates local state
    itself ; it calls :class:`AdminClient` for cone evolution and
    :class:`CatalogBridge` for binding lookup. The caller supplies an
    optional :class:`AuditEmitter` so the outcome can be persisted
    through M11.
    """

    admin: AdminClient
    catalog: CatalogBridge
    audit: AuditEmitter | None = None
    relax_factor: float = 1.2
    min_relax_severity: DriftSeverity = DriftSeverity.major

    async def react(
        self,
        result: DriftResult | DriftEvent,
        *,
        actor_id: str = "spiffe://efe.local/prod/drift",
    ) -> DriftReactionOutcome:
        event = (
            result
            if isinstance(result, DriftEvent)
            else drift_result_to_event(result)
        )
        # Skip if severity under the threshold.
        if _severity_rank(event.severity) < _severity_rank(self.min_relax_severity):
            outcome = DriftReactionOutcome(event=event, action="noop")
            await self._maybe_audit(event, outcome)
            return outcome

        # Resolve cone binding ; if absent, we can't evolve.
        try:
            cone_ref: ConeRef = await self.catalog.resolve(
                tenant_id=event.tenant_id,
                feature_id=event.feature_name,
                task_id=event.task_id,
            )
        except ConeRefNotBound:
            outcome = DriftReactionOutcome(
                event=event, action="noop", error="no_binding"
            )
            await self._maybe_audit(event, outcome)
            return outcome

        # Compute the relaxed radius and issue an evolve.
        # Reasonable default radius ; a production caller can override.
        target_radius = 0.35 * self.relax_factor
        try:
            handle = await self.admin.evolve_cone(
                from_key=cone_ref.key,
                op=update_radius_op(target_radius),
                tenant_id=event.tenant_id,
                actor_id=actor_id,
            )
            outcome = DriftReactionOutcome(
                event=event,
                action="cone_relaxed",
                new_cone_key=handle.key,
            )
        except Exception as exc:  # noqa: BLE001
            outcome = DriftReactionOutcome(
                event=event,
                action="error",
                error=str(exc),
            )
        await self._maybe_audit(event, outcome)
        return outcome

    async def _maybe_audit(
        self, event: DriftEvent, outcome: DriftReactionOutcome
    ) -> None:
        if self.audit is not None:
            try:
                await self.audit(event, outcome)
            except Exception:  # pragma: no cover - audit must not break
                pass


def _severity_rank(s: DriftSeverity) -> int:
    return {
        DriftSeverity.none: 0,
        DriftSeverity.warning: 1,
        DriftSeverity.major: 2,
        DriftSeverity.critical: 3,
    }[s]


_ = field, Any, timedelta  # keep imports available for future hooks
