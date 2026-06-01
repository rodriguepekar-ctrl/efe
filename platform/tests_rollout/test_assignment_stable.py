"""Invariant I1 — assignment determinism + I8 holdout exclusion."""

from __future__ import annotations


import pytest

from efe_rollout.assignment import (
    HashAssigner,
    HoldoutFilter,
    InMemoryAssignmentCache,
    InMemoryForcedAssignmentStore,
)
from efe_rollout.assignment.forced import ForcedAssignment
from efe_rollout.experiment import ExperimentSpec, Variant
from efe_rollout.layers import LayerRegistry


def _spec_with_ids() -> ExperimentSpec:
    return ExperimentSpec(
        name="assign-test",
        tenant_id="acme",
        task_id="summarize",
        layer="prompt",
        kind="ab_fixed",
        owner="alice",
        hypothesis="h",
        primary_metric="ctr",
        variants=(
            Variant(
                name="control", allocation_pct=50, is_control=True,
                variant_id="v-control",
            ),
            Variant(
                name="treatment", allocation_pct=50,
                variant_id="v-treatment",
            ),
        ),
    )


@pytest.mark.asyncio
async def test_assignment_is_stable_for_same_user():
    spec = _spec_with_ids()
    assigner = HashAssigner(
        layers=LayerRegistry.default(),
        holdout=HoldoutFilter(0),   # disable holdout to see pure hash
    )
    first = await assigner.assign(
        user_id="u-1", experiment_id="exp-1", spec=spec,
    )
    # Clear cache via new assigner with fresh cache — still deterministic.
    assigner2 = HashAssigner(
        layers=LayerRegistry.default(),
        holdout=HoldoutFilter(0),
        cache=InMemoryAssignmentCache(),
    )
    second = await assigner2.assign(
        user_id="u-1", experiment_id="exp-1", spec=spec,
    )
    assert first.variant is not None and second.variant is not None
    assert first.variant.variant_id == second.variant.variant_id


@pytest.mark.asyncio
async def test_assignment_covers_both_variants_roughly_balanced():
    spec = _spec_with_ids()
    assigner = HashAssigner(
        layers=LayerRegistry.default(),
        holdout=HoldoutFilter(0),
    )
    counts = {"v-control": 0, "v-treatment": 0}
    for i in range(10_000):
        d = await assigner.assign(
            user_id=f"u-{i}", experiment_id="exp-1", spec=spec,
        )
        assert d.variant is not None
        counts[d.variant.variant_id] += 1
    # 50/50 split with 10k samples: SE ≈ 50 → expect each ∈ [4700, 5300]
    assert 4_700 <= counts["v-control"] <= 5_300
    assert 4_700 <= counts["v-treatment"] <= 5_300


@pytest.mark.asyncio
async def test_cache_hit_returns_same_decision_quickly():
    spec = _spec_with_ids()
    cache = InMemoryAssignmentCache()
    assigner = HashAssigner(
        layers=LayerRegistry.default(),
        holdout=HoldoutFilter(0),
        cache=cache,
    )
    first = await assigner.assign(
        user_id="u-42", experiment_id="exp-x", spec=spec,
    )
    second = await assigner.assign(
        user_id="u-42", experiment_id="exp-x", spec=spec,
    )
    assert second.from_cache
    assert first.variant and second.variant
    assert first.variant.variant_id == second.variant.variant_id


@pytest.mark.asyncio
async def test_forced_overrides_holdout():
    spec = _spec_with_ids()
    forced = InMemoryForcedAssignmentStore([
        ForcedAssignment(
            experiment_id="exp-1",
            user_id="qa-alice",
            variant_id="v-treatment",
            reason="QA",
            created_by="ops",
        ),
    ])
    assigner = HashAssigner(
        layers=LayerRegistry.default(),
        holdout=HoldoutFilter(10_000),   # everyone in holdout
        forced=forced,
    )
    d = await assigner.assign(
        user_id="qa-alice", experiment_id="exp-1", spec=spec,
    )
    assert d.forced
    assert not d.holdout
    assert d.variant is not None
    assert d.variant.variant_id == "v-treatment"


@pytest.mark.asyncio
async def test_holdout_returns_no_variant():
    spec = _spec_with_ids()
    assigner = HashAssigner(
        layers=LayerRegistry.default(),
        holdout=HoldoutFilter(10_000),
    )
    d = await assigner.assign(
        user_id="u-17", experiment_id="exp-1", spec=spec,
    )
    assert d.holdout
    assert d.variant is None


@pytest.mark.asyncio
async def test_holdout_is_about_1_percent_on_default_config():
    spec = _spec_with_ids()
    assigner = HashAssigner(
        layers=LayerRegistry.default(),
        holdout=HoldoutFilter(100),  # 1 %
    )
    holdout_count = 0
    for i in range(50_000):
        d = await assigner.assign(
            user_id=f"u-{i}", experiment_id="exp-1", spec=spec,
        )
        if d.holdout:
            holdout_count += 1
    # 1 % of 50 000 ≈ 500. Allow ±150 for noise.
    assert 350 <= holdout_count <= 650


@pytest.mark.asyncio
async def test_outside_traffic_cap_returns_no_variant():
    spec = ExperimentSpec(
        name="sub",
        tenant_id="acme",
        task_id="summarize",
        layer="prompt",
        kind="ab_fixed",
        owner="alice",
        hypothesis="h",
        primary_metric="ctr",
        traffic_allocation_bps=1_000,   # only 10 % of users on this exp
        variants=(
            Variant(name="control", allocation_pct=50, is_control=True,
                    variant_id="a"),
            Variant(name="treatment", allocation_pct=50, variant_id="b"),
        ),
    )
    assigner = HashAssigner(
        layers=LayerRegistry.default(),
        holdout=HoldoutFilter(0),
    )
    outside = 0
    total = 20_000
    for i in range(total):
        d = await assigner.assign(
            user_id=f"u-{i}", experiment_id="exp", spec=spec,
        )
        if d.outside_traffic:
            outside += 1
    # 90 % outside ± 1 %
    assert 0.85 * total <= outside <= 0.95 * total
