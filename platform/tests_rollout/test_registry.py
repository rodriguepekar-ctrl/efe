"""Experiment registry — FSM transitions + pre-reg + layer slots."""

from __future__ import annotations

import pytest

from efe_rollout.clock import FakeClock
from efe_rollout.errors import (
    DuplicateExperiment,
    ExperimentNotFound,
    InvalidTransition,
    LayerSlotBusy,
    PreRegistrationMissing,
    PreRegistrationSealed,
)
from efe_rollout.experiment import ExperimentSpec, Variant
from efe_rollout.registry import ExperimentRegistry, InMemoryRegistryStore


def _spec(name: str = "exp", **ov) -> ExperimentSpec:
    base = dict(
        name=name,
        tenant_id="acme",
        task_id="summarize",
        layer="prompt",
        kind="ab_fixed",
        owner="alice",
        hypothesis="h",
        primary_metric="ctr",
        variants=(
            Variant(name="control", allocation_pct=50, is_control=True),
            Variant(name="treatment", allocation_pct=50),
        ),
    )
    base.update(ov)
    return ExperimentSpec(**base)


def _registry(clock: FakeClock) -> ExperimentRegistry:
    return ExperimentRegistry(
        store=InMemoryRegistryStore(),
        signing_key=b"k" * 32,
        signing_key_id="kms/alias-1",
        clock=clock,
    )


@pytest.mark.asyncio
async def test_create_and_fetch():
    reg = _registry(FakeClock())
    exp = await reg.create(_spec("e1"))
    again = await reg.get(exp.experiment_id)
    assert again.spec.name == "e1"
    assert again.state == "draft"


@pytest.mark.asyncio
async def test_duplicate_name_rejected():
    reg = _registry(FakeClock())
    await reg.create(_spec("e1"))
    with pytest.raises(DuplicateExperiment):
        await reg.create(_spec("e1"))


@pytest.mark.asyncio
async def test_transition_happy_path():
    reg = _registry(FakeClock())
    exp = await reg.create(_spec("e1"))
    exp2 = await reg.transition(
        exp.experiment_id, to_state="designed", actor_role="owner",
    )
    assert exp2.state == "designed"


@pytest.mark.asyncio
async def test_transition_invalid_rejected():
    reg = _registry(FakeClock())
    exp = await reg.create(_spec("e1"))
    with pytest.raises(InvalidTransition):
        await reg.transition(
            exp.experiment_id, to_state="running", actor_role="owner",
        )


@pytest.mark.asyncio
async def test_pre_register_happy_path():
    reg = _registry(FakeClock())
    exp = await reg.create(_spec("e1"))
    await reg.transition(
        exp.experiment_id, to_state="designed", actor_role="owner",
    )
    prereg = await reg.pre_register(exp.experiment_id)
    after = await reg.get(exp.experiment_id)
    assert prereg.experiment_id == exp.experiment_id
    assert after.state == "registered"


@pytest.mark.asyncio
async def test_pre_register_requires_designed_state():
    reg = _registry(FakeClock())
    exp = await reg.create(_spec("e1"))
    with pytest.raises(PreRegistrationMissing):
        await reg.pre_register(exp.experiment_id)


@pytest.mark.asyncio
async def test_pre_register_is_idempotent_and_sealed():
    reg = _registry(FakeClock())
    exp = await reg.create(_spec("e1"))
    await reg.transition(
        exp.experiment_id, to_state="designed", actor_role="owner",
    )
    await reg.pre_register(exp.experiment_id)
    with pytest.raises(PreRegistrationSealed):
        await reg.pre_register(exp.experiment_id)


@pytest.mark.asyncio
async def test_layer_slot_busy_rejected():
    reg = _registry(FakeClock())
    a = await reg.create(_spec("a"))
    b = await reg.create(_spec("b"))
    # Move A through the full state machine to RUNNING.
    await reg.transition(a.experiment_id, to_state="designed",
                          actor_role="owner")
    await reg.pre_register(a.experiment_id)
    await reg.transition(a.experiment_id, to_state="running",
                          actor_role="owner")
    # Now try B on the same (tenant, task, layer) slot.
    await reg.transition(b.experiment_id, to_state="designed",
                          actor_role="owner")
    await reg.pre_register(b.experiment_id)
    with pytest.raises(LayerSlotBusy):
        await reg.transition(b.experiment_id, to_state="running",
                              actor_role="owner")


@pytest.mark.asyncio
async def test_layer_slot_freed_after_terminal():
    reg = _registry(FakeClock())
    a = await reg.create(_spec("a"))
    await reg.transition(a.experiment_id, to_state="designed",
                          actor_role="owner")
    await reg.pre_register(a.experiment_id)
    await reg.transition(a.experiment_id, to_state="running",
                          actor_role="owner")
    # Terminate A.
    await reg.transition(a.experiment_id, to_state="aborted",
                          actor_role="admin")
    # Now B can take the slot.
    b = await reg.create(_spec("b"))
    await reg.transition(b.experiment_id, to_state="designed",
                          actor_role="owner")
    await reg.pre_register(b.experiment_id)
    nxt = await reg.transition(b.experiment_id, to_state="running",
                                actor_role="owner")
    assert nxt.state == "running"


@pytest.mark.asyncio
async def test_get_unknown_experiment_raises():
    reg = _registry(FakeClock())
    with pytest.raises(ExperimentNotFound):
        await reg.get("does-not-exist")


@pytest.mark.asyncio
async def test_verify_prereg():
    reg = _registry(FakeClock())
    exp = await reg.create(_spec("e1"))
    await reg.transition(
        exp.experiment_id, to_state="designed", actor_role="owner",
    )
    await reg.pre_register(exp.experiment_id)
    pr = await reg.verify_prereg(exp.experiment_id)
    assert pr.experiment_id == exp.experiment_id
