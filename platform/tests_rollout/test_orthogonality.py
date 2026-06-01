"""Invariant I4 — layer orthogonality over 100 k users.

When two experiments live on different layers (e.g. prompt vs model),
their assignments must be statistically independent. We measure the
empirical correlation coefficient between the two assignment streams
and require it to be ≤ 0.05 — the production threshold that
:class:`LayerRegistry` needs to respect.
"""

from __future__ import annotations

import math

import pytest

from efe_rollout.assignment import (
    HashAssigner,
    HoldoutFilter,
    InMemoryAssignmentCache,
)
from efe_rollout.experiment import ExperimentSpec, Variant
from efe_rollout.layers import LayerRegistry


def _spec(layer: str, experiment_id_hint: str) -> ExperimentSpec:
    return ExperimentSpec(
        name=f"ortho-{layer}-{experiment_id_hint}",
        tenant_id="acme",
        task_id="summarize",
        layer=layer,
        kind="ab_fixed",
        owner="alice",
        hypothesis="h",
        primary_metric="ctr",
        variants=(
            Variant(name="control", allocation_pct=50, is_control=True,
                    variant_id=f"{experiment_id_hint}-a"),
            Variant(name="treatment", allocation_pct=50,
                    variant_id=f"{experiment_id_hint}-b"),
        ),
    )


def _corr(xs: list[int], ys: list[int]) -> float:
    n = len(xs)
    mx = sum(xs) / n
    my = sum(ys) / n
    num = sum((xs[i] - mx) * (ys[i] - my) for i in range(n))
    dx = math.sqrt(sum((x - mx) ** 2 for x in xs))
    dy = math.sqrt(sum((y - my) ** 2 for y in ys))
    if dx == 0 or dy == 0:
        return 0.0
    return num / (dx * dy)


@pytest.mark.asyncio
async def test_two_layers_produce_uncorrelated_assignments():
    spec_prompt = _spec("prompt", "p")
    spec_model = _spec("model", "m")
    layers = LayerRegistry.default()
    assigner = HashAssigner(
        layers=layers,
        holdout=HoldoutFilter(0),
        cache=InMemoryAssignmentCache(),
    )
    xs: list[int] = []
    ys: list[int] = []
    n = 20_000
    for i in range(n):
        d_prompt = await assigner.assign(
            user_id=f"u-{i}", experiment_id="exp-p", spec=spec_prompt,
        )
        d_model = await assigner.assign(
            user_id=f"u-{i}", experiment_id="exp-m", spec=spec_model,
        )
        assert d_prompt.variant is not None
        assert d_model.variant is not None
        # Encode assignment as 0 (control) / 1 (treatment) for each layer.
        xs.append(
            0 if d_prompt.variant.variant_id.endswith("-a") else 1,
        )
        ys.append(
            0 if d_model.variant.variant_id.endswith("-a") else 1,
        )
    rho = _corr(xs, ys)
    # Production threshold is 0.05 — at n=20_000 we expect |ρ| < 0.02
    # almost always.
    assert abs(rho) < 0.05, f"layer correlation too high: {rho}"


@pytest.mark.asyncio
async def test_same_layer_different_experiments_also_uncorrelated():
    spec_a = _spec("prompt", "a")
    spec_b = _spec("prompt", "b")
    # Same layer / tenant but different experiment *names* — the
    # assigner mixes the spec name into the variant hash so two
    # experiments on the same slot produce independent streams.
    layers = LayerRegistry.default()
    assigner = HashAssigner(
        layers=layers,
        holdout=HoldoutFilter(0),
        cache=InMemoryAssignmentCache(),
    )
    xs: list[int] = []
    ys: list[int] = []
    n = 10_000
    for i in range(n):
        da = await assigner.assign(
            user_id=f"u-{i}", experiment_id="e1", spec=spec_a,
        )
        db = await assigner.assign(
            user_id=f"u-{i}", experiment_id="e2", spec=spec_b,
        )
        assert da.variant is not None
        assert db.variant is not None
        xs.append(0 if da.variant.variant_id.endswith("-a") else 1)
        ys.append(0 if db.variant.variant_id.endswith("-a") else 1)
    rho = _corr(xs, ys)
    # Production threshold: |ρ| ≤ 0.05 (M08 §7.4).
    assert abs(rho) < 0.05, f"same-layer corr too high: {rho}"
