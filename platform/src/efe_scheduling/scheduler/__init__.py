"""Scheduler subsystem — time-based firing.

Public API :

* :class:`CronExpression`, :class:`IntervalEvaluator`,
  :class:`OneShotEvaluator`, :class:`RRuleEvaluator` — deterministic
  ``iter_fires`` implementations.
* :class:`SchedulerEngine` — the main orchestration loop.
* :class:`LeaderElection` — cooperative leader election.
* :func:`fire_key_for` — build the HMAC idempotency key.
"""

from __future__ import annotations

from efe_scheduling.scheduler.cron import CronExpression, parse_cron
from efe_scheduling.scheduler.engine import SchedulerEngine, SchedulerEngineConfig
from efe_scheduling.scheduler.evaluators import (
    CompositeEvaluator,
    IntervalEvaluator,
    OneShotEvaluator,
    RRuleEvaluator,
    ScheduleEvaluator,
    build_evaluator,
)
from efe_scheduling.scheduler.fire_key import fire_key_for
from efe_scheduling.scheduler.leader import InMemoryLeaderElection, LeaderElection

__all__ = [
    "CompositeEvaluator",
    "CronExpression",
    "InMemoryLeaderElection",
    "IntervalEvaluator",
    "LeaderElection",
    "OneShotEvaluator",
    "RRuleEvaluator",
    "ScheduleEvaluator",
    "SchedulerEngine",
    "SchedulerEngineConfig",
    "build_evaluator",
    "fire_key_for",
    "parse_cron",
]
