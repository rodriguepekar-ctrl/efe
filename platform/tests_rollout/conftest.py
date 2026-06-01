"""Shared fixtures for the rollout test suite."""

from __future__ import annotations

import pytest

from efe_rollout.clock import FakeClock
from efe_rollout.experiment import ExperimentSpec, GuardrailSpec, Variant


@pytest.fixture
def fake_clock() -> FakeClock:
    return FakeClock()


@pytest.fixture
def basic_spec() -> ExperimentSpec:
    return ExperimentSpec(
        name="summarize-v2",
        tenant_id="acme",
        task_id="summarize",
        layer="prompt",
        kind="ab_fixed",
        owner="alice@example.com",
        hypothesis="three-shot beats zero-shot",
        primary_metric="thumbs_up_rate",
        variants=(
            Variant(name="control", allocation_pct=50, is_control=True,
                    payload={"prompt_id": "zero-shot"}),
            Variant(name="treatment", allocation_pct=50,
                    payload={"prompt_id": "three-shot"}),
        ),
        guardrails=(
            GuardrailSpec(
                metric_name="latency_p95_ms",
                direction="up",
                threshold_pct=30.0,
            ),
        ),
        minimum_detectable_effect=0.05,
    )
