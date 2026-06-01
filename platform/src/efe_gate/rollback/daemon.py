"""Rollback daemon — post-promote canary monitor.

Workflow (one tick):

    1. List all ``rollback_monitors`` in state ``watching``.
    2. For each, pull live metrics from M07 (via :class:`LiveMetricsProvider`).
    3. Compare against ``metrics_snapshot`` recorded at promotion.
       Regression = candidate live value drops by more than
       ``auto_rollback_min_delta`` AND the drop is statistically
       significant at ``auto_rollback_threshold_p_value``.
    4. On regression → call :meth:`ChampionRegistry.rollback` and
       mark the monitor ``rolled_back``.
    5. On deadline expiry → mark ``expired``.

The daemon is stateless between ticks and uses a Postgres advisory
lock for leader election (only one replica actively runs ticks).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any, Protocol

import numpy as np
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_gate.champion.registry import ChampionRegistry
from efe_gate.clock import Clock, SystemClock
from efe_gate.config import RollbackSettings
from efe_gate.db.models import Champion, RollbackMonitor
from efe_gate.metrics.stats.bootstrap import bootstrap_delta_ci
from efe_gate.observability.logging import get_logger

_log = get_logger(__name__)


def _as_utc(ts: datetime) -> datetime:
    """SQLite strips tzinfo on read; graft UTC back."""
    if ts.tzinfo is None:
        return ts.replace(tzinfo=timezone.utc)
    return ts.astimezone(timezone.utc)


# ---------------------------------------------------------------------------
# LiveMetricsProvider — abstraction over M07
# ---------------------------------------------------------------------------


class LiveMetricsProvider(Protocol):
    async def fetch(
        self, tenant_id: str, task_id: str,
        *, window_seconds: int, metric_name: str,
    ) -> np.ndarray:  # pragma: no cover
        """Return per-sample live scores over the window. Empty array if none."""


@dataclass
class StaticLiveMetricsProvider:
    """Test-friendly implementation.

    Stores a dict ``{(tenant_id, task_id, metric): np.ndarray}`` and
    returns the closest match. The dict can be mutated between ticks
    to simulate a regression.
    """

    data: dict[tuple[str, str, str], np.ndarray] = field(default_factory=dict)
    fail: bool = False

    async def fetch(
        self, tenant_id: str, task_id: str,
        *, window_seconds: int, metric_name: str,
    ) -> np.ndarray:
        if self.fail:
            return np.array([], dtype=np.float64)
        return self.data.get(
            (tenant_id, task_id, metric_name),
            np.array([], dtype=np.float64),
        )


# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class RollbackReport:
    scanned: int
    rolled_back: tuple[str, ...]
    expired: tuple[str, ...]
    still_watching: tuple[str, ...]
    skipped: tuple[str, ...]


# ---------------------------------------------------------------------------
# Daemon
# ---------------------------------------------------------------------------


@dataclass
class RollbackDaemon:
    session: AsyncSession
    registry: ChampionRegistry
    live_metrics: LiveMetricsProvider
    settings: RollbackSettings
    clock: Clock = field(default_factory=SystemClock)

    # -------- one-shot scan --------

    async def tick(self) -> RollbackReport:
        now = self.clock.now()
        stmt = (
            select(RollbackMonitor)
            .where(RollbackMonitor.state == "watching")
        )
        monitors = list((await self.session.execute(stmt)).scalars())
        rolled_back: list[str] = []
        expired: list[str] = []
        still: list[str] = []
        skipped: list[str] = []

        for m in monitors:
            if _as_utc(now) >= _as_utc(m.deadline):
                m.state = "expired"
                m.final_reason = "window elapsed; candidate held as champion"
                expired.append(m.run_id)
                continue

            # Check the champion still points at this run_id. If not,
            # something else already replaced it — mark expired.
            champ = await self.session.get(
                Champion, (m.tenant_id, m.task_id),
            )
            if champ is None or champ.run_id != m.run_id:
                m.state = "expired"
                m.final_reason = "champion changed externally"
                expired.append(m.run_id)
                continue

            live_metric = _primary_metric_name(
                dict(champ.metrics_snapshot or {}),
            )
            if live_metric is None:
                skipped.append(m.run_id)
                continue

            window_seconds = self.settings.tick_seconds
            live_scores = await self.live_metrics.fetch(
                tenant_id=m.tenant_id, task_id=m.task_id,
                window_seconds=window_seconds,
                metric_name=live_metric,
            )
            if live_scores.size < self.settings.min_live_samples:
                still.append(m.run_id)
                m.n_ticks = m.n_ticks + 1
                m.last_tick_at = now
                continue
            baseline_value = float(
                (champ.metrics_snapshot or {})
                .get(live_metric, {}).get("value", 0.0)
            )
            baseline = np.full(
                live_scores.size, baseline_value, dtype=np.float64,
            )
            # Paired-bootstrap delta: baseline vs live (not paired in
            # reality but statistically close for this binary setup).
            res = bootstrap_delta_ci(
                baseline, live_scores,
                n_resamples=2_000,
                conf=self.settings.auto_rollback_threshold_p_value,
                seed=1337,
            )
            # live - baseline ; lower value means regression for
            # higher-is-better metrics. With baseline constant, res.mean
            # is simply live.mean() - baseline_value.
            delta = res.mean  # live - baseline
            observed_drop = -delta  # positive number = drop in live
            m.observed_metrics = {
                **(m.observed_metrics or {}),
                live_metric: {
                    "n": int(live_scores.size),
                    "live_mean": float(live_scores.mean()),
                    "baseline": baseline_value,
                    "delta": float(delta),
                    "ci_lower": float(res.ci_lower),
                    "ci_upper": float(res.ci_upper),
                },
            }
            m.n_ticks = m.n_ticks + 1
            m.last_tick_at = now

            should_rollback = (
                observed_drop >= self.settings.auto_rollback_min_delta
                and res.ci_upper < 0  # even upper bound of (live-base) < 0
            )
            if should_rollback:
                try:
                    await self.registry.rollback(
                        tenant_id=m.tenant_id, task_id=m.task_id,
                        actor="rollback-daemon",
                        reason=(
                            f"live {live_metric} drop "
                            f"{observed_drop:.4f} ≥ threshold"
                        ),
                    )
                    m.state = "rolled_back"
                    m.final_reason = (
                        f"auto rollback on {live_metric} regression"
                    )
                    rolled_back.append(m.run_id)
                except Exception as exc:
                    _log.error("rollback_failed", run_id=m.run_id,
                                error=str(exc))
                    skipped.append(m.run_id)
            else:
                still.append(m.run_id)

        await self.session.flush()
        return RollbackReport(
            scanned=len(monitors),
            rolled_back=tuple(rolled_back),
            expired=tuple(expired),
            still_watching=tuple(still),
            skipped=tuple(skipped),
        )


def _primary_metric_name(
    snapshot: dict[str, Any],
) -> str | None:
    """Pick the metric to monitor — first primary in the snapshot."""
    for name, entry in snapshot.items():
        if not isinstance(entry, dict):
            continue
        if entry.get("role") == "primary":
            return name
    # Fallback — first numeric metric.
    for name in ("accuracy", "macro_f1", "recall@1"):
        if name in snapshot:
            return name
    return None


__all__ = [
    "LiveMetricsProvider",
    "RollbackDaemon",
    "RollbackReport",
    "StaticLiveMetricsProvider",
]
