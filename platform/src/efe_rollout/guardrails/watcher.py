"""Guardrail watcher — rolling-window regression detector.

Each guardrail is a ``(metric, direction, threshold_pct, window_s,
consecutive_breaches)`` tuple that compares treatment vs control over
a rolling window. A "breach" is detected when :

  direction="up"   and ratio ≥ 1 + threshold_pct/100
  direction="down" and ratio ≤ 1 - threshold_pct/100

After ``consecutive_breaches`` breaches the watcher triggers
:class:`AutoAbortExecutor`.

Two design choices worth keeping in mind :

* **Aging** — after 24 h of no breach the consecutive-breach counter
  resets; we don't want random noise to accumulate breaches over weeks.

* **Re-check on each tick** — like M06's canary we never commit to an
  abort based on stale data. The watcher recomputes the ratio every
  ``tick_period_s`` seconds.
"""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass
from datetime import datetime, timedelta

from efe_rollout.clock import Clock, SystemClock
from efe_rollout.experiment import GuardrailSpec
from efe_rollout.records import Outcome


@dataclass(slots=True)
class GuardrailRuntime:
    """Mutable per-experiment guardrail runtime state."""

    spec: GuardrailSpec
    breaches_so_far: int = 0
    last_check_at: datetime | None = None
    last_breach_at: datetime | None = None
    last_ratio: float | None = None


@dataclass(frozen=True, slots=True)
class GuardrailTickResult:
    breached: bool
    trips_abort: bool             # cumulative breach count crossed the threshold
    ratio: float | None
    reason: str


class GuardrailWatcher:
    """Pure-logic watcher — no NATS, no DB.

    Callers feed it the latest windowed outcomes (split by control vs
    treatment) and it returns a :class:`GuardrailTickResult` per
    guardrail + a decision flag. Auto-abort dispatch is left to
    :class:`AutoAbortExecutor`.
    """

    __slots__ = ("_aging_s", "_clock")

    def __init__(
        self,
        *,
        aging_s: int = 86_400,
        clock: Clock | None = None,
    ) -> None:
        self._aging_s = int(aging_s)
        self._clock = clock or SystemClock()

    def tick(
        self,
        runtime: GuardrailRuntime,
        *,
        control_outcomes: Iterable[Outcome],
        treatment_outcomes: Iterable[Outcome],
    ) -> GuardrailTickResult:
        now = self._clock.now()
        spec = runtime.spec

        # Step 0 — age-out BEFORE the breach check so a long quiet
        # period between ticks still resets the counter. Previously
        # the reset only fired on a non-breach tick, which meant a
        # ``breach → 24 h silence → breach`` pattern incorrectly kept
        # the counter at 2 instead of 1.
        if (
            runtime.last_breach_at is not None
            and (now - runtime.last_breach_at).total_seconds() >= self._aging_s
        ):
            runtime.breaches_so_far = 0

        control_vals = [
            o.metric_value for o in control_outcomes
            if o.metric_name == spec.metric_name
            and self._inside_window(o.observed_at, now, spec.window_s)
        ]
        treatment_vals = [
            o.metric_value for o in treatment_outcomes
            if o.metric_name == spec.metric_name
            and self._inside_window(o.observed_at, now, spec.window_s)
        ]
        # Not enough data for a decision — keep breach counter intact.
        if not control_vals or not treatment_vals:
            runtime.last_check_at = now
            return GuardrailTickResult(
                breached=False,
                trips_abort=False,
                ratio=None,
                reason="insufficient_window_data",
            )
        mean_control = sum(control_vals) / len(control_vals)
        mean_treatment = sum(treatment_vals) / len(treatment_vals)
        if mean_control == 0.0:
            # Zero baseline — can't compute a relative ratio for the
            # "down" direction (no way to regress below zero), but for
            # "up" a non-zero treatment IS a regression from zero.
            if spec.direction == "down":
                runtime.last_check_at = now
                return GuardrailTickResult(
                    breached=False,
                    trips_abort=False,
                    ratio=None,
                    reason="zero_baseline_no_ratio",
                )
            ratio = float("inf") if mean_treatment > 0 else 1.0
        else:
            ratio = mean_treatment / mean_control
        runtime.last_ratio = ratio
        runtime.last_check_at = now
        is_breach = self._is_breach(ratio, spec)
        if is_breach:
            runtime.breaches_so_far += 1
            runtime.last_breach_at = now
            trips = runtime.breaches_so_far >= spec.consecutive_breaches
            return GuardrailTickResult(
                breached=True,
                trips_abort=trips,
                ratio=ratio,
                reason=(
                    f"breach {runtime.breaches_so_far}/"
                    f"{spec.consecutive_breaches}"
                ),
            )
        return GuardrailTickResult(
            breached=False,
            trips_abort=False,
            ratio=ratio,
            reason="ratio_within_bounds",
        )

    # ------------------------------------------------------------------
    # helpers
    # ------------------------------------------------------------------

    def _is_breach(self, ratio: float, spec: GuardrailSpec) -> bool:
        threshold = spec.threshold_pct / 100.0
        if spec.direction == "up":
            return ratio >= (1.0 + threshold)
        return ratio <= (1.0 - threshold)

    def _inside_window(
        self, observed_at: datetime, now: datetime, window_s: int,
    ) -> bool:
        return now - timedelta(seconds=window_s) <= observed_at <= now


__all__ = [
    "GuardrailRuntime",
    "GuardrailTickResult",
    "GuardrailWatcher",
]
