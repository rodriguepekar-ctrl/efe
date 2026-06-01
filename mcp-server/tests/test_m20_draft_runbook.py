"""efe_draft_runbook (M20) — pin tests."""
from __future__ import annotations

import json

import pytest

from m20_helpers import make_runbook_response




@pytest.mark.asyncio
async def test_draft_runbook_basic(
    m20_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = make_runbook_response()
    result = await m20_dispatcher.dispatch(
        tool_name="efe_draft_runbook",
        args={
            "incident_event": {
                "slo_id": "M19-OBS-3",
                "trigger_metric": "log_volume_kbytes_per_minute",
                "current_value": 247,
                "threshold": 100,
            },
            "related_logs": ["WARN otel-collector latency p99 = 8s"],
        },
    )
    assert result.is_error is False
    body = result.content
    assert len(body["runbook_markdown"]) >= 200
    assert body["risk_level"] in ("low", "medium", "high")
    assert 1 <= body["estimated_resolution_time_min"] <= 240


@pytest.mark.asyncio
async def test_draft_runbook_redacts_pii_in_logs(
    m20_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = make_runbook_response()
    await m20_dispatcher.dispatch(
        tool_name="efe_draft_runbook",
        args={
            "incident_event": {"slo_id": "M19-OBS-3"},
            "related_logs": [
                "ERROR auth failed for user@evil.com",
            ],
        },
    )
    call = fake_sampling_session.calls[-1]
    prompt_text = ""
    for m in call["messages"]:
        c = m.get("content")
        if isinstance(c, dict):
            prompt_text += c.get("text", "")
    assert "user@evil.com" not in prompt_text
    assert "[REDACTED:" in prompt_text


@pytest.mark.asyncio
async def test_draft_runbook_validates_eta_range(
    m20_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = json.dumps({
        "runbook_markdown": "x" * 250,
        "estimated_resolution_time_min": 9999,  # out of [1, 240]
        "primary_diagnostics": ["d"],
        "risk_level": "low",
    })
    result = await m20_dispatcher.dispatch(
        tool_name="efe_draft_runbook",
        args={"incident_event": {"slo_id": "M19-OBS-3"}},
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_draft_runbook_validates_risk_enum(
    m20_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = json.dumps({
        "runbook_markdown": "x" * 250,
        "estimated_resolution_time_min": 30,
        "primary_diagnostics": ["d"],
        "risk_level": "catastrophic",  # invalid
    })
    result = await m20_dispatcher.dispatch(
        tool_name="efe_draft_runbook",
        args={"incident_event": {"slo_id": "M19-OBS-3"}},
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_draft_runbook_optional_logs(
    m20_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = make_runbook_response()
    result = await m20_dispatcher.dispatch(
        tool_name="efe_draft_runbook",
        args={"incident_event": {"slo_id": "M19-OBS-3"}},
    )
    assert result.is_error is False
    assert result.content["metadata"]["n_logs_referenced"] == 0


@pytest.mark.asyncio
async def test_draft_runbook_includes_draft_warning(
    m20_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = make_runbook_response()
    result = await m20_dispatcher.dispatch(
        tool_name="efe_draft_runbook",
        args={"incident_event": {"slo_id": "M19-OBS-3"}},
    )
    assert "DRAFT" in result.content["metadata"]["draft_warning"].upper()


@pytest.mark.asyncio
async def test_draft_runbook_blocks_when_feature_disabled(
    m20_off_dispatcher, fake_sampling_session,
):
    result = await m20_off_dispatcher.dispatch(
        tool_name="efe_draft_runbook",
        args={"incident_event": {"slo_id": "M19-OBS-3"}},
    )
    assert result.is_error is True
