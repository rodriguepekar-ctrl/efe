"""Tests for :class:`SpecBuilder` — invariants and edge cases."""

from __future__ import annotations

import math

import pytest

from efe_platform.errors import SwarmInvalidSpec
from efe_platform.swarm.protocol import (
    AnchorRef,
    ConeRef,
    ConvergenceThresholds,
    PopulationEntry,
    ResourceBudget,
)
from efe_platform.swarm.spec_builder import SpecBuilder


def _embedding(n: int = 16) -> tuple[float, ...]:
    return tuple(float(i) / n for i in range(n))


# ---------------------------------------------------------------------------
#  Happy path
# ---------------------------------------------------------------------------


def test_minimal_valid_spec() -> None:
    spec = (
        SpecBuilder()
        .with_problem_embedding(_embedding())
        .with_cone_ref(ConeRef("acme", "task", 1))
        .build()
    )
    assert spec.dim_latent == 16
    assert spec.alignment_cone_ref == ConeRef("acme", "task", 1)
    assert spec.alignment_cone is None
    # Default population totals 1024.
    total = sum(p.count for p in spec.population)
    assert total == 1024


def test_builder_is_immutable() -> None:
    b1 = SpecBuilder().with_problem_embedding(_embedding())
    b2 = b1.with_max_rounds(4)
    # b1 unaffected.
    spec1 = b1.with_cone_ref(ConeRef("t", "k", 1)).build()
    spec2 = b2.with_cone_ref(ConeRef("t", "k", 1)).build()
    assert spec1.max_rounds == 8  # default
    assert spec2.max_rounds == 4


def test_default_population_shortcut_sums_to_1024() -> None:
    b = SpecBuilder().with_default_population()
    assert sum(p.count for p in b._population) == 1024
    # Three templates.
    assert {p.template_code for p in b._population} == {"na-sem", "na-log", "na-crit"}


# ---------------------------------------------------------------------------
#  Embedding validation
# ---------------------------------------------------------------------------


def test_empty_embedding_rejected() -> None:
    with pytest.raises(SwarmInvalidSpec, match="must not be empty"):
        SpecBuilder().with_problem_embedding([])


def test_dim_latent_mismatch_rejected() -> None:
    with pytest.raises(SwarmInvalidSpec, match="dim_latent"):
        SpecBuilder().with_problem_embedding(_embedding(16), dim_latent=32)


def test_non_finite_rejected() -> None:
    v = list(_embedding(16))
    v[5] = math.nan
    with pytest.raises(SwarmInvalidSpec, match="non-finite"):
        SpecBuilder().with_problem_embedding(v)


def test_too_small_dim_latent_rejected() -> None:
    with pytest.raises(SwarmInvalidSpec, match="dim_latent must be in"):
        SpecBuilder().with_problem_embedding(_embedding(4))


def test_too_large_dim_latent_rejected() -> None:
    with pytest.raises(SwarmInvalidSpec):
        SpecBuilder().with_problem_embedding(_embedding(100_000))


# ---------------------------------------------------------------------------
#  Population validation
# ---------------------------------------------------------------------------


def test_unknown_template_rejected() -> None:
    with pytest.raises(SwarmInvalidSpec, match="unknown template"):
        SpecBuilder().with_population([("na-bogus", 10)])


def test_duplicate_template_rejected() -> None:
    with pytest.raises(SwarmInvalidSpec, match="duplicate template"):
        SpecBuilder().with_population([("na-sem", 10), ("na-sem", 5)])


def test_total_too_small_rejected() -> None:
    with pytest.raises(SwarmInvalidSpec, match="population total must be in"):
        SpecBuilder().with_population([("na-sem", 4)])


def test_total_too_large_rejected() -> None:
    with pytest.raises(SwarmInvalidSpec, match="population total must be in"):
        SpecBuilder().with_population([("na-sem", 50_000)])


def test_population_entry_rejects_zero() -> None:
    with pytest.raises(ValueError, match="count must be > 0"):
        PopulationEntry("na-sem", 0)


# ---------------------------------------------------------------------------
#  Cone source validation
# ---------------------------------------------------------------------------


def test_missing_cone_source_rejected() -> None:
    with pytest.raises(SwarmInvalidSpec, match="exactly one of cone_anchors or cone_ref"):
        SpecBuilder().with_problem_embedding(_embedding()).build()


def test_both_cone_sources_rejected() -> None:
    # Explicitly: set anchors then ref — the setter resets anchors; set
    # both by poking internals to test the .build() guard.
    b = (
        SpecBuilder()
        .with_problem_embedding(_embedding())
        .with_cone_anchors([[0.0] * 16])
    )
    b._cone_ref = ConeRef("a", "b", 1)  # force-set both
    with pytest.raises(SwarmInvalidSpec, match="exactly one"):
        b.build()


def test_anchor_dim_mismatch_rejected() -> None:
    b = (
        SpecBuilder()
        .with_problem_embedding(_embedding(16))
        .with_cone_anchors([[0.0] * 8])
    )
    with pytest.raises(SwarmInvalidSpec, match="!= dim_latent"):
        b.build()


def test_anchor_dimensions_consistent() -> None:
    with pytest.raises(SwarmInvalidSpec, match="must all match"):
        SpecBuilder().with_cone_anchors([[0.0] * 8, [0.0] * 16])


# ---------------------------------------------------------------------------
#  Budget / rounds
# ---------------------------------------------------------------------------


def test_budget_too_small_rejected() -> None:
    b = (
        SpecBuilder()
        .with_problem_embedding(_embedding())
        .with_cone_ref(ConeRef("a", "b", 1))
        .with_max_rounds(32)
        .with_budget(ResourceBudget(max_wall_clock_ms=5))
    )
    with pytest.raises(SwarmInvalidSpec, match="too small for max_rounds"):
        b.build()


def test_custom_convergence() -> None:
    c = ConvergenceThresholds(
        epsilon_variance=0.001, epsilon_gradient=0.001, epsilon_stability=0.005
    )
    spec = (
        SpecBuilder()
        .with_problem_embedding(_embedding())
        .with_cone_ref(ConeRef("t", "k", 1))
        .with_convergence(c)
        .build()
    )
    assert spec.convergence is c


def test_convergence_rejects_out_of_range() -> None:
    with pytest.raises(ValueError, match="epsilon"):
        ConvergenceThresholds(epsilon_variance=1.5)


def test_anchor_ref_rejects_empty() -> None:
    with pytest.raises(ValueError, match="non-empty"):
        AnchorRef(())


def test_cone_ref_key_format() -> None:
    r = ConeRef("acme", "route", 7)
    assert r.key() == "acme/route@v7"
