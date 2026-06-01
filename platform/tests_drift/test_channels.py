"""Slack, PagerDuty, Webhook, NATS channels."""

from __future__ import annotations

from datetime import datetime, timezone
from typing import Any

import pytest

from efe_drift.alerting.channels import (
    NatsAlertChannel,
    PagerDutyChannel,
    SlackChannel,
    WebhookChannel,
)
from efe_drift.alerting.channels.base import (
    IncidentPayload,
    build_incident_payload,
)
from efe_drift.alerting.incident import AlertIncident
from efe_drift.alerting.rule import AlertRule, ChannelRef
from efe_drift.clock import FakeClock
from efe_drift.detection import SeverityThresholds
from efe_drift.detection.base import DriftResult
from efe_drift.hashing import (
    DOMAIN_TAG_NATS,
    canonical_json_bytes,
    hmac_verify,
)
from efe_drift.ids import new_uuid
from efe_drift.nats.publisher import InMemoryPublisher


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


def _payload() -> IncidentPayload:
    result = DriftResult(
        evaluation_id=new_uuid(),
        tenant_id="acme", task_id="summarize",
        feature_id="f1", feature_name="prompt_length",
        baseline_id="b1",
        window_start=datetime(2026, 4, 10, 10, 0, tzinfo=timezone.utc),
        window_end=datetime(2026, 4, 10, 11, 0, tzinfo=timezone.utc),
        method="psi", score=0.42, severity="critical",
        computed_at=datetime(2026, 4, 10, 11, 0, tzinfo=timezone.utc),
        multi_scale_window="1h",
        p_value=None,
    )
    rule = AlertRule(
        rule_id="r1",
        tenant_id="acme",
        method="psi",
        thresholds=SeverityThresholds(
            warning=0.1, major=0.25, critical=0.4,
        ),
        channels=(ChannelRef(channel="whatever"),),
    )
    incident = AlertIncident(
        incident_id="i1", rule_id="r1",
        tenant_id="acme", feature_id="f1", feature_name="prompt_length",
        method="psi", severity="critical", score_at_fire=0.42,
        fired_at=datetime(2026, 4, 10, 11, 0, tzinfo=timezone.utc),
        window_kind="1h",
    )
    return build_incident_payload(
        incident=incident, result=result, rule=rule,
    )


# ---------------------------------------------------------------------------
# Slack
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_slack_channel_posts_blocks():
    captured: dict[str, Any] = {}

    async def poster(url: str, body: dict[str, Any]) -> tuple[int, str]:
        captured["url"] = url
        captured["body"] = body
        return 200, "ok"

    clock = FakeClock()
    channel = SlackChannel(
        webhook_url="https://hooks.slack.com/services/X/Y/Z",
        http_poster=poster, mention="@drift", clock=clock,
    )
    result = await channel.dispatch(_payload())
    assert result.ok
    assert result.channel == "slack"
    assert "blocks" in captured["body"]
    header = captured["body"]["blocks"][0]["text"]["text"]
    assert "CRITICAL" in header


@pytest.mark.asyncio
async def test_slack_channel_handles_non_200():
    async def poster(url: str, body: dict[str, Any]) -> tuple[int, str]:
        return 500, "server error"

    channel = SlackChannel(
        webhook_url="https://hooks.slack.com/services/X/Y/Z",
        http_poster=poster, clock=FakeClock(),
    )
    result = await channel.dispatch(_payload())
    assert not result.ok
    assert "500" in (result.error or "")


@pytest.mark.asyncio
async def test_slack_channel_handles_exception():
    async def poster(url: str, body: dict[str, Any]) -> tuple[int, str]:
        raise ConnectionError("down")

    channel = SlackChannel(
        webhook_url="x", http_poster=poster, clock=FakeClock(),
    )
    result = await channel.dispatch(_payload())
    assert not result.ok
    assert "ConnectionError" in (result.error or "")


# ---------------------------------------------------------------------------
# PagerDuty
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_pagerduty_channel_happy_path():
    captured: dict[str, Any] = {}

    async def poster(url: str, body: dict[str, Any]) -> tuple[int, str]:
        captured["url"] = url
        captured["body"] = body
        return 202, "Accepted"

    channel = PagerDutyChannel(
        routing_key="RKEY",
        http_poster=poster,
        clock=FakeClock(),
    )
    result = await channel.dispatch(_payload())
    assert result.ok
    assert result.channel == "pagerduty"
    assert captured["body"]["routing_key"] == "RKEY"
    assert captured["body"]["event_action"] == "trigger"
    assert captured["body"]["payload"]["severity"] == "critical"
    # Dedup key is deterministic.
    dedup = captured["body"]["dedup_key"]
    assert "acme:summarize:prompt_length" in dedup


@pytest.mark.asyncio
async def test_pagerduty_map_severity():
    from efe_drift.alerting.channels.pagerduty import _map_severity
    assert _map_severity("critical") == "critical"
    assert _map_severity("major") == "error"
    assert _map_severity("warning") == "warning"


# ---------------------------------------------------------------------------
# Webhook
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_webhook_channel_signs_body():
    key = b"k" * 32
    captured: dict[str, Any] = {}

    async def poster(
        url: str, body: dict[str, Any], headers: dict[str, str],
    ) -> tuple[int, str]:
        captured["headers"] = headers
        captured["body"] = body
        return 200, ""

    channel = WebhookChannel(
        url="https://siem.example/alert",
        hmac_key=key,
        http_poster=poster,
        clock=FakeClock(),
    )
    payload = _payload()
    result = await channel.dispatch(payload)
    assert result.ok
    sig_hex = captured["headers"]["X-EFE-Drift-Signature"]
    # Verify.
    sig = bytes.fromhex(sig_hex)
    raw = canonical_json_bytes(payload.to_dict())
    assert hmac_verify(raw, key, sig, domain_tag=DOMAIN_TAG_NATS)


@pytest.mark.asyncio
async def test_webhook_channel_retries_on_5xx_then_gives_up():
    attempts = {"n": 0}

    async def poster(
        url: str, body: dict[str, Any], headers: dict[str, str],
    ) -> tuple[int, str]:
        attempts["n"] += 1
        return 503, "try again later"

    channel = WebhookChannel(
        url="x", hmac_key=b"k" * 32,
        http_poster=poster, max_retries=3, backoff_s=0.0,
        clock=FakeClock(),
    )
    result = await channel.dispatch(_payload())
    assert not result.ok
    assert attempts["n"] == 3


@pytest.mark.asyncio
async def test_webhook_channel_4xx_stops_immediately():
    attempts = {"n": 0}

    async def poster(
        url: str, body: dict[str, Any], headers: dict[str, str],
    ) -> tuple[int, str]:
        attempts["n"] += 1
        return 400, "bad request"

    channel = WebhookChannel(
        url="x", hmac_key=b"k" * 32,
        http_poster=poster, max_retries=3, backoff_s=0.0,
        clock=FakeClock(),
    )
    result = await channel.dispatch(_payload())
    assert not result.ok
    assert attempts["n"] == 1


# ---------------------------------------------------------------------------
# NATS
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_nats_channel_publishes_payload():
    publisher = InMemoryPublisher()
    channel = NatsAlertChannel(
        publisher=publisher,
        subject="efe.drift.alert",
        clock=FakeClock(),
    )
    result = await channel.dispatch(_payload())
    assert result.ok
    assert result.channel == "nats"
    msgs = publisher.messages(subject="efe.drift.alert")
    assert len(msgs) == 1
    assert msgs[0][1]["severity"] == "critical"


@pytest.mark.asyncio
async def test_nats_channel_handles_publisher_failure():
    class _FailingPublisher:
        async def publish(self, subject: str, payload: dict[str, Any]) -> None:
            raise RuntimeError("down")

    channel = NatsAlertChannel(
        publisher=_FailingPublisher(),  # type: ignore[arg-type]
        clock=FakeClock(),
    )
    result = await channel.dispatch(_payload())
    assert not result.ok
    assert "RuntimeError" in (result.error or "")
