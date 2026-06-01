"""AlertRouter — hysteresis + suppression + dispatch.

Flow :

1. For each :class:`DriftResult`, find matching rules via
   :meth:`AlertRuleStore.matching_rules`.
2. Per (rule × result) pair :
   * If ``result.is_breach()`` → increment per-``(rule, feature,
     severity)`` counter.
   * When the counter hits ``rule.consecutive_breaches`` AND no
     suppression lock is active AND no active (firing/acked) incident
     already exists → **fire** : create :class:`AlertIncident` +
     dispatch to each :class:`AlertChannel` whose ``min_severity`` is
     met.
   * If the result is NOT a breach AND an active incident exists →
     resolve it.
3. Per-fire the router sets a suppression lock until
   ``fired_at + rule.suppression_s``. Subsequent breaches within that
   window increment the counter but **do not** dispatch.

Hysteresis state is held in a small in-memory dict keyed by
``(rule_id, feature_id, severity)``. A DB-backed
:class:`HysteresisState` can be swapped in without touching the
router.

Every dispatch attempt is recorded in the incident's
``dispatch_results`` ledger, even when it fails.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from typing import Protocol

from efe_drift.alerting.channels.base import (
    ChannelRegistry,
    DispatchResult,
    build_incident_payload,
)
from efe_drift.alerting.incident import (
    AlertIncident,
    AlertIncidentStore,
)
from efe_drift.alerting.rule import AlertRule, AlertRuleStore, ChannelRef
from efe_drift.clock import Clock, SystemClock
from efe_drift.detection.base import DriftResult
from efe_drift.detection.severity import Severity

# ---------------------------------------------------------------------------
# Hysteresis + suppression state
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class _RuleState:
    breach_count: int = 0
    last_breach_at: datetime | None = None
    suppressed_until: datetime | None = None
    active_incident_id: str | None = None


class HysteresisState(Protocol):
    async def get(
        self, *, rule_id: str, feature_id: str, severity: Severity,
    ) -> _RuleState:  # pragma: no cover
        ...

    async def set(
        self,
        *,
        rule_id: str,
        feature_id: str,
        severity: Severity,
        state: _RuleState,
    ) -> None:  # pragma: no cover
        ...

    async def clear(
        self, *, rule_id: str, feature_id: str, severity: Severity,
    ) -> None:  # pragma: no cover
        ...


class InMemoryHysteresisState:
    __slots__ = ("_lock", "_states")

    def __init__(self) -> None:
        self._states: dict[tuple[str, str, str], _RuleState] = {}
        self._lock = asyncio.Lock()

    def _key(
        self, *, rule_id: str, feature_id: str, severity: Severity,
    ) -> tuple[str, str, str]:
        return (rule_id, feature_id, severity)

    async def get(
        self, *, rule_id: str, feature_id: str, severity: Severity,
    ) -> _RuleState:
        async with self._lock:
            key = self._key(
                rule_id=rule_id, feature_id=feature_id, severity=severity,
            )
            if key not in self._states:
                self._states[key] = _RuleState()
            return self._states[key]

    async def set(
        self,
        *,
        rule_id: str,
        feature_id: str,
        severity: Severity,
        state: _RuleState,
    ) -> None:
        async with self._lock:
            key = self._key(
                rule_id=rule_id, feature_id=feature_id, severity=severity,
            )
            self._states[key] = state

    async def clear(
        self, *, rule_id: str, feature_id: str, severity: Severity,
    ) -> None:
        async with self._lock:
            key = self._key(
                rule_id=rule_id, feature_id=feature_id, severity=severity,
            )
            self._states.pop(key, None)


# ---------------------------------------------------------------------------
# RoutingDecision — pure audit record
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class RoutingDecision:
    rule_id: str
    result: DriftResult
    fired: bool
    suppressed: bool
    resolved: bool
    incident: AlertIncident | None
    dispatches: tuple[DispatchResult, ...] = field(default_factory=tuple)
    note: str | None = None


# ---------------------------------------------------------------------------
# AlertRouter
# ---------------------------------------------------------------------------


SEVERITY_RANK: dict[str, int] = {
    "none": 0, "warning": 1, "major": 2, "critical": 3,
}


class AlertRouter:
    __slots__ = (
        "_channels",
        "_clock",
        "_hysteresis",
        "_incidents",
        "_rules",
    )

    def __init__(
        self,
        *,
        rules: AlertRuleStore,
        incidents: AlertIncidentStore,
        channels: ChannelRegistry,
        hysteresis: HysteresisState | None = None,
        clock: Clock | None = None,
    ) -> None:
        self._rules = rules
        self._incidents = incidents
        self._channels = channels
        self._hysteresis = hysteresis or InMemoryHysteresisState()
        self._clock = clock or SystemClock()

    async def handle(
        self, result: DriftResult,
    ) -> tuple[RoutingDecision, ...]:
        """Process one drift result against every matching rule."""
        rules = await self._rules.matching_rules(result)
        decisions: list[RoutingDecision] = []
        for rule in rules:
            decisions.append(await self._handle_rule(rule, result))
        return tuple(decisions)

    async def handle_many(
        self, results: tuple[DriftResult, ...],
    ) -> tuple[RoutingDecision, ...]:
        all_decisions: list[RoutingDecision] = []
        for r in results:
            all_decisions.extend(await self.handle(r))
        return tuple(all_decisions)

    # ---- internals -----------------------------------------------------

    async def _handle_rule(
        self, rule: AlertRule, result: DriftResult,
    ) -> RoutingDecision:
        # Re-classify the result using the rule's own thresholds.
        severity = rule.thresholds.classify(result.score)
        now = self._clock.now()
        # Re-compose the result so downstream (payload / incident)
        # carries the rule-local severity.
        scoped_result = _with_severity(result, severity)
        if severity == "none":
            return await self._handle_no_breach(rule, scoped_result, now)
        return await self._handle_breach(rule, scoped_result, severity, now)

    async def _handle_no_breach(
        self, rule: AlertRule, result: DriftResult, now: datetime,
    ) -> RoutingDecision:
        # Resolve active incidents across **all** severities for this
        # (rule, feature) — keeps the state machine clean.
        resolved = False
        for sev in ("warning", "major", "critical"):
            state = await self._hysteresis.get(
                rule_id=rule.rule_id,
                feature_id=result.feature_id,
                severity=sev,
            )
            if state.active_incident_id is not None:
                await self._incidents.resolve(
                    state.active_incident_id, now=now,
                )
                await self._hysteresis.clear(
                    rule_id=rule.rule_id,
                    feature_id=result.feature_id,
                    severity=sev,
                )
                resolved = True
        return RoutingDecision(
            rule_id=rule.rule_id,
            result=result,
            fired=False,
            suppressed=False,
            resolved=resolved,
            incident=None,
            note="no_breach",
        )

    async def _handle_breach(
        self,
        rule: AlertRule,
        result: DriftResult,
        severity: Severity,
        now: datetime,
    ) -> RoutingDecision:
        state = await self._hysteresis.get(
            rule_id=rule.rule_id,
            feature_id=result.feature_id,
            severity=severity,
        )
        state.breach_count += 1
        state.last_breach_at = now
        # Suppressed — keep counting but do not dispatch.
        if (
            state.suppressed_until is not None
            and now < state.suppressed_until
        ):
            await self._hysteresis.set(
                rule_id=rule.rule_id,
                feature_id=result.feature_id,
                severity=severity,
                state=state,
            )
            return RoutingDecision(
                rule_id=rule.rule_id,
                result=result,
                fired=False,
                suppressed=True,
                resolved=False,
                incident=(
                    await self._maybe_incident(state.active_incident_id)
                ),
                note="suppressed",
            )
        # Hysteresis threshold not yet crossed.
        if state.breach_count < rule.consecutive_breaches:
            await self._hysteresis.set(
                rule_id=rule.rule_id,
                feature_id=result.feature_id,
                severity=severity,
                state=state,
            )
            return RoutingDecision(
                rule_id=rule.rule_id,
                result=result,
                fired=False,
                suppressed=False,
                resolved=False,
                incident=None,
                note=(
                    f"hysteresis {state.breach_count}/"
                    f"{rule.consecutive_breaches}"
                ),
            )
        # An incident may already be active for this (rule, feature,
        # severity) — don't open a duplicate.
        if state.active_incident_id is not None:
            incident = await self._incidents.get(state.active_incident_id)
            await self._hysteresis.set(
                rule_id=rule.rule_id,
                feature_id=result.feature_id,
                severity=severity,
                state=state,
            )
            return RoutingDecision(
                rule_id=rule.rule_id,
                result=result,
                fired=False,
                suppressed=False,
                resolved=False,
                incident=incident,
                note="already_firing",
            )
        # Fire a new incident.
        incident = await self._incidents.create(
            rule_id=rule.rule_id, result=result, now=now,
        )
        state.active_incident_id = incident.incident_id
        state.suppressed_until = now + timedelta(seconds=rule.suppression_s)
        await self._hysteresis.set(
            rule_id=rule.rule_id,
            feature_id=result.feature_id,
            severity=severity,
            state=state,
        )
        dispatches = await self._dispatch(
            rule=rule, incident=incident, result=result,
        )
        # Persist dispatch results on the incident.
        for d in dispatches:
            incident = await self._incidents.record_dispatch(
                incident.incident_id,
                channel=d.channel,
                result=d.to_dict(),
            )
        return RoutingDecision(
            rule_id=rule.rule_id,
            result=result,
            fired=True,
            suppressed=False,
            resolved=False,
            incident=incident,
            dispatches=dispatches,
            note="fired",
        )

    async def _dispatch(
        self,
        *,
        rule: AlertRule,
        incident: AlertIncident,
        result: DriftResult,
    ) -> tuple[DispatchResult, ...]:
        payload = build_incident_payload(
            incident=incident, result=result, rule=rule,
        )
        out: list[DispatchResult] = []
        for chan_ref in rule.channels:
            if not self._should_route(chan_ref, incident.severity):
                continue
            try:
                channel = self._channels.get(chan_ref.channel)
            except KeyError:
                out.append(DispatchResult(
                    ok=False, channel=chan_ref.channel,
                    error="channel_not_registered",
                ))
                continue
            result_ = await channel.dispatch(payload)
            out.append(result_)
        return tuple(out)

    @staticmethod
    def _should_route(ref: ChannelRef, severity: str) -> bool:
        return SEVERITY_RANK.get(severity, 0) >= SEVERITY_RANK.get(
            ref.min_severity, 0,
        )

    async def _maybe_incident(
        self, incident_id: str | None,
    ) -> AlertIncident | None:
        if incident_id is None:
            return None
        try:
            return await self._incidents.get(incident_id)
        except Exception:
            return None


# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------


def _with_severity(
    result: DriftResult, severity: Severity,
) -> DriftResult:
    from dataclasses import replace
    if result.severity == severity:
        return result
    return replace(result, severity=severity)


__all__ = [
    "AlertRouter",
    "HysteresisState",
    "InMemoryHysteresisState",
    "RoutingDecision",
]
