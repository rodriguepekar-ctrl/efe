"""Triggers subsystem — reactive firing.

Exports :

* :class:`Trigger`, :class:`TriggerContext`, :class:`Decision`,
  :class:`Fire`, :class:`Hold`, :class:`Reset` — the protocol.
* :class:`Debouncer` — anti-flap state machine.
* :class:`DriftThresholdTrigger`, :class:`MetricTrigger`,
  :class:`ManualTrigger`, :class:`AllOfTrigger`, :class:`AnyOfTrigger`,
  :class:`NotTrigger` — concrete triggers.
"""

from __future__ import annotations

from efe_scheduling.triggers.base import (
    Decision,
    Fire,
    Hold,
    Reset,
    Trigger,
    TriggerContext,
)
from efe_scheduling.triggers.composite import (
    AllOfTrigger,
    AnyOfTrigger,
    NotTrigger,
)
from efe_scheduling.triggers.debouncer import Debouncer, DebouncerConfig, DebouncerState
from efe_scheduling.triggers.drift import (
    DriftEvent,
    DriftSeverity,
    DriftThresholdTrigger,
    DriftTriggerSpec,
)
from efe_scheduling.triggers.manual import ManualTrigger
from efe_scheduling.triggers.metric import (
    MetricSample,
    MetricTrigger,
    MetricTriggerSpec,
)

__all__ = [
    "AllOfTrigger",
    "AnyOfTrigger",
    "Debouncer",
    "DebouncerConfig",
    "DebouncerState",
    "Decision",
    "DriftEvent",
    "DriftSeverity",
    "DriftThresholdTrigger",
    "DriftTriggerSpec",
    "Fire",
    "Hold",
    "ManualTrigger",
    "MetricSample",
    "MetricTrigger",
    "MetricTriggerSpec",
    "NotTrigger",
    "Reset",
    "Trigger",
    "TriggerContext",
]
