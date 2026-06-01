"""Top-level SDK — RolloutClient happy paths."""

from __future__ import annotations

import pytest

from efe_rollout.adapter import RolloutClient
from efe_rollout.errors import ExperimentNotRunning
from efe_rollout.experiment import ExperimentSpec, Variant
from efe_rollout.metrics.exposure import InMemoryExposureSink
from efe_rollout.metrics.outcome import InMemoryOutcomeSink


def _spec(**ov) -> ExperimentSpec:
    base = dict(
        name="summarize-v2",
        tenant_id="acme",
        task_id="summarize",
        layer="prompt",
        kind="ab_fixed",
        owner="alice",
        hypothesis="h",
        primary_metric="ctr",
        variants=(
            Variant(name="control", allocation_pct=50, is_control=True,
                    payload={"prompt_id": "zero"}),
            Variant(name="treatment", allocation_pct=50,
                    payload={"prompt_id": "three"}),
        ),
    )
    base.update(ov)
    return ExperimentSpec(**base)


@pytest.mark.asyncio
async def test_get_variant_requires_running_state():
    client = RolloutClient.in_memory()
    exp = await client.registry.create(_spec())
    with pytest.raises(ExperimentNotRunning):
        await client.get_variant(
            user_id="u-1", experiment_id=exp.experiment_id,
        )


@pytest.mark.asyncio
async def test_end_to_end_assignment_and_outcome():
    expo_sink = InMemoryExposureSink()
    out_sink = InMemoryOutcomeSink()
    client = RolloutClient.in_memory(
        exposure_sink=expo_sink, outcome_sink=out_sink,
    )
    exp = await client.registry.create(_spec())
    await client.registry.transition(
        exp.experiment_id, to_state="designed", actor_role="owner",
    )
    await client.registry.pre_register(exp.experiment_id)
    await client.registry.transition(
        exp.experiment_id, to_state="running", actor_role="owner",
    )
    # Get variant
    res = await client.get_variant(
        user_id="u-123", experiment_id=exp.experiment_id,
    )
    assert res.variant_id is not None
    # One exposure was recorded.
    assert len(expo_sink.all()) == 1
    assert expo_sink.all()[0].user_id == "u-123"
    # Record an outcome.
    await client.record_outcome(
        experiment_id=exp.experiment_id,
        user_id="u-123",
        metric_name="ctr",
        metric_value=1.0,
        variant_id=res.variant_id,
    )
    assert len(out_sink.all()) == 1
    assert out_sink.all()[0].metric_value == 1.0


@pytest.mark.asyncio
async def test_holdout_user_gets_none_variant_id():
    from efe_rollout.config import AssignmentSettings, RolloutSettings

    settings = RolloutSettings(
        assignment=AssignmentSettings(holdout_basis_points=10_000),
    )
    client = RolloutClient.in_memory(settings=settings)
    exp = await client.registry.create(_spec())
    await client.registry.transition(
        exp.experiment_id, to_state="designed", actor_role="owner",
    )
    await client.registry.pre_register(exp.experiment_id)
    await client.registry.transition(
        exp.experiment_id, to_state="running", actor_role="owner",
    )
    res = await client.get_variant(
        user_id="held-out-user", experiment_id=exp.experiment_id,
    )
    assert res.variant_id is None
    assert res.holdout is True
