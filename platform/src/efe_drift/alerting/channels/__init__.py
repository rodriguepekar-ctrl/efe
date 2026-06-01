"""Alert channels — Slack, PagerDuty, webhook, NATS + registry."""

from __future__ import annotations

from efe_drift.alerting.channels.base import (
    AlertChannel,
    ChannelRegistry,
    DispatchResult,
    IncidentPayload,
    build_incident_payload,
)
from efe_drift.alerting.channels.nats import NatsAlertChannel
from efe_drift.alerting.channels.pagerduty import PagerDutyChannel
from efe_drift.alerting.channels.slack import SlackChannel
from efe_drift.alerting.channels.webhook import WebhookChannel

__all__ = [
    "AlertChannel",
    "ChannelRegistry",
    "DispatchResult",
    "IncidentPayload",
    "NatsAlertChannel",
    "PagerDutyChannel",
    "SlackChannel",
    "WebhookChannel",
    "build_incident_payload",
]
