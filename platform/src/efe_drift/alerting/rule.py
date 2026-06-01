"""Alert rule — links a ``(tenant, feature, method, window)`` scope to
a severity ladder, a hysteresis policy, and a routing config.

Matching semantics :

* ``tenant_id`` is **required** — rules are never cross-tenant.
* ``task_id = None`` means "any task for this tenant".
* ``feature_id = None`` means "any feature matching the method".
* ``method`` must match the :class:`DriftResult.method` exactly.
* ``window_kind`` (``"1h"`` / ``"1d"`` / ``"7d"``) filters the
  multi-scale window.

When several rules match a given result, the most-specific one wins
(``feature_id`` set beats ``feature_id = None``; likewise for
``task_id``). See :meth:`AlertRuleStore.matching_rules` for the
ordering.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, replace
from datetime import datetime
from typing import Protocol

from efe_drift.clock import Clock, SystemClock
from efe_drift.detection.base import DriftMethod, DriftResult, MultiScaleWindow
from efe_drift.detection.severity import SeverityThresholds
from efe_drift.errors import NotFound, ValidationError
from efe_drift.ids import new_uuid

# ---------------------------------------------------------------------------
# Channel reference (resolved against a ChannelRegistry at dispatch)
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class ChannelRef:
    """Reference to an alert channel by its registry key + min severity.

    A rule can route to multiple channels (e.g. Slack on ``major+``,
    PagerDuty on ``critical`` only, NATS on ``warning+``).
    """

    channel: str                          # e.g. "slack", "pagerduty"
    min_severity: str = "warning"         # warning / major / critical

    def __post_init__(self) -> None:
        if self.min_severity not in {"warning", "major", "critical"}:
            raise ValidationError(
                f"min_severity must be warning/major/critical, "
                f"got {self.min_severity!r}",
            )


# ---------------------------------------------------------------------------
# AlertRule
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class AlertRule:
    """Immutable alert rule record."""

    rule_id: str
    tenant_id: str
    method: DriftMethod
    thresholds: SeverityThresholds
    channels: tuple[ChannelRef, ...]
    task_id: str | None = None
    feature_id: str | None = None
    window_kind: MultiScaleWindow | None = None
    consecutive_breaches: int = 3
    suppression_s: int = 1800
    enabled: bool = True
    name: str | None = None
    notes: str | None = None
    created_at: datetime | None = None

    def __post_init__(self) -> None:
        if self.consecutive_breaches < 1:
            raise ValidationError(
                f"consecutive_breaches must be ≥ 1, "
                f"got {self.consecutive_breaches}",
            )
        if self.suppression_s < 0:
            raise ValidationError(
                f"suppression_s must be ≥ 0, got {self.suppression_s}",
            )
        if not self.channels:
            raise ValidationError("rule must define ≥ 1 channel")

    def matches(self, result: DriftResult) -> bool:
        if not self.enabled:
            return False
        if self.tenant_id != result.tenant_id:
            return False
        if self.method != result.method:
            return False
        if self.task_id is not None and self.task_id != result.task_id:
            return False
        if (
            self.feature_id is not None
            and self.feature_id != result.feature_id
        ):
            return False
        return not (
            self.window_kind is not None
            and self.window_kind != result.multi_scale_window
        )

    @property
    def specificity(self) -> int:
        """Higher score = more specific rule; used for tie-breaking."""
        score = 0
        if self.task_id is not None:
            score += 1
        if self.feature_id is not None:
            score += 2
        if self.window_kind is not None:
            score += 1
        return score


# ---------------------------------------------------------------------------
# AlertRuleStore Protocol + in-memory impl
# ---------------------------------------------------------------------------


class AlertRuleStore(Protocol):
    async def put(self, rule: AlertRule) -> AlertRule:  # pragma: no cover
        ...

    async def get(self, rule_id: str) -> AlertRule:  # pragma: no cover
        ...

    async def delete(self, rule_id: str) -> None:  # pragma: no cover
        ...

    async def list(
        self, *, tenant_id: str | None = None,
    ) -> tuple[AlertRule, ...]:  # pragma: no cover
        ...

    async def matching_rules(
        self, result: DriftResult,
    ) -> tuple[AlertRule, ...]:  # pragma: no cover
        ...


class InMemoryAlertRuleStore:
    """Lock-guarded in-memory rule store for tests / single-pod dev."""

    __slots__ = ("_by_id", "_clock", "_lock")

    def __init__(self, *, clock: Clock | None = None) -> None:
        self._by_id: dict[str, AlertRule] = {}
        self._clock = clock or SystemClock()
        self._lock = asyncio.Lock()

    # ---- CRUD ----------------------------------------------------------

    async def put(self, rule: AlertRule) -> AlertRule:
        async with self._lock:
            if rule.created_at is None:
                rule = replace(rule, created_at=self._clock.now())
            self._by_id[rule.rule_id] = rule
            return rule

    async def create(
        self,
        *,
        tenant_id: str,
        method: DriftMethod,
        thresholds: SeverityThresholds,
        channels: tuple[ChannelRef, ...],
        task_id: str | None = None,
        feature_id: str | None = None,
        window_kind: MultiScaleWindow | None = None,
        consecutive_breaches: int = 3,
        suppression_s: int = 1800,
        name: str | None = None,
        notes: str | None = None,
    ) -> AlertRule:
        rule = AlertRule(
            rule_id=new_uuid(),
            tenant_id=tenant_id,
            method=method,
            thresholds=thresholds,
            channels=channels,
            task_id=task_id,
            feature_id=feature_id,
            window_kind=window_kind,
            consecutive_breaches=consecutive_breaches,
            suppression_s=suppression_s,
            name=name,
            notes=notes,
            created_at=self._clock.now(),
        )
        return await self.put(rule)

    async def get(self, rule_id: str) -> AlertRule:
        rule = self._by_id.get(rule_id)
        if rule is None:
            raise NotFound(f"unknown rule_id {rule_id!r}")
        return rule

    async def delete(self, rule_id: str) -> None:
        async with self._lock:
            self._by_id.pop(rule_id, None)

    # ---- queries --------------------------------------------------------

    async def list(
        self, *, tenant_id: str | None = None,
    ) -> tuple[AlertRule, ...]:
        rules = tuple(self._by_id.values())
        if tenant_id is not None:
            rules = tuple(r for r in rules if r.tenant_id == tenant_id)
        return rules

    async def matching_rules(
        self, result: DriftResult,
    ) -> tuple[AlertRule, ...]:
        """Return matching rules sorted by specificity (descending)."""
        matches = [
            rule for rule in self._by_id.values() if rule.matches(result)
        ]
        matches.sort(key=lambda r: r.specificity, reverse=True)
        return tuple(matches)


__all__ = [
    "AlertRule",
    "AlertRuleStore",
    "ChannelRef",
    "InMemoryAlertRuleStore",
]
