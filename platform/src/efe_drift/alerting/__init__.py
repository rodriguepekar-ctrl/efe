"""Alerting plane — L5."""

from __future__ import annotations

from efe_drift.alerting.channels import (
    AlertChannel,
    ChannelRegistry,
    DispatchResult,
    IncidentPayload,
    NatsAlertChannel,
    PagerDutyChannel,
    SlackChannel,
    WebhookChannel,
    build_incident_payload,
)
from efe_drift.alerting.incident import (
    AlertIncident,
    AlertIncidentStore,
    IncidentFsm,
    IncidentState,
    InMemoryAlertIncidentStore,
)
from efe_drift.alerting.router import (
    AlertRouter,
    HysteresisState,
    InMemoryHysteresisState,
    RoutingDecision,
)
from efe_drift.alerting.rule import (
    AlertRule,
    AlertRuleStore,
    ChannelRef,
    InMemoryAlertRuleStore,
)

__all__ = [
    "AlertChannel",
    "AlertIncident",
    "AlertIncidentStore",
    "AlertRouter",
    "AlertRule",
    "AlertRuleStore",
    "ChannelRef",
    "ChannelRegistry",
    "DispatchResult",
    "HysteresisState",
    "InMemoryAlertIncidentStore",
    "InMemoryAlertRuleStore",
    "InMemoryHysteresisState",
    "IncidentFsm",
    "IncidentPayload",
    "IncidentState",
    "NatsAlertChannel",
    "PagerDutyChannel",
    "RoutingDecision",
    "SlackChannel",
    "WebhookChannel",
    "build_incident_payload",
]
