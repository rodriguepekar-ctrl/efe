"""Shared fixtures for the drift test suite."""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from efe_drift.clock import FakeClock
from efe_drift.records import InferenceEvent


@pytest.fixture
def fake_clock() -> FakeClock:
    return FakeClock()


@pytest.fixture
def sample_event() -> InferenceEvent:
    return InferenceEvent(
        event_id="e-1",
        tenant_id="acme",
        task_id="summarize",
        user_id="user-42",
        observed_at=datetime(2026, 4, 20, 10, 0, 0, tzinfo=timezone.utc),
        request_id="req-1",
        prompt_token_count=120,
        response_token_count=60,
        latency_ms=150.0,
        cost_usd=0.0023,
        model_id="llama-7b",
        backend_kind="vllm",
        temperature=0.7,
        top_p=0.95,
        prompt_text="Summarise the quarterly earnings call in 3 bullets.",
        response_text="• Revenue up 12%\n• Margins expanded\n• Guidance raised",
        covariates={"country": "FR", "device": "mobile"},
    )
