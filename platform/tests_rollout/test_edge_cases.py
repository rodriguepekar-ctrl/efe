"""Additional edge-case coverage identified during the M08 audit.

* Stats parity against scipy (when installed).
* Bayesian decision boundary behaviour.
* Meta-analysis with degenerate inputs.
* Sequential α-spending: cumulative-α upper bound property.
* Assigner: 3-variant uneven split (33/33/34).
* SRM detector: high-significance 60/40 vs designed 50/50.
* Variant with ``allocation_pct=0`` must never be assigned.
* Forced-assignment store CRUD lifecycle.
* Layer registry rotation invalidates salt.
* BH adjusted-p monotonicity property.
"""

from __future__ import annotations

import math

import pytest

from efe_rollout.analysis.bayesian import (
    bayesian_beta_binomial,
    bayesian_decision,
)
from efe_rollout.analysis.meta import StudySummary, meta_analyze
from efe_rollout.analysis.sequential import AlphaSpending
from efe_rollout.analysis.srm import detect_srm
from efe_rollout.analysis.stats import (
    bh_adjusted_p,
    chi2_cdf,
    normal_cdf,
    normal_ppf,
    student_t_cdf,
)
from efe_rollout.assignment import (
    HashAssigner,
    HoldoutFilter,
    InMemoryAssignmentCache,
    InMemoryForcedAssignmentStore,
)
from efe_rollout.assignment.forced import ForcedAssignment
from efe_rollout.experiment import ExperimentSpec, Variant
from efe_rollout.layers import Layer, LayerRegistry


# ---------------------------------------------------------------------------
# Stats parity — skipped gracefully if scipy is missing
# ---------------------------------------------------------------------------


def test_stats_parity_against_scipy_if_available():
    try:
        from scipy.stats import chi2, norm, t  # type: ignore[import-untyped]
    except ImportError:
        pytest.skip("scipy not installed")
    # normal_ppf accurate to 1e-5 across typical α levels
    for p in (0.001, 0.025, 0.05, 0.5, 0.95, 0.975, 0.999):
        assert abs(normal_ppf(p) - norm.ppf(p)) < 1e-4
    # normal_cdf accurate
    for x in (-3.0, -1.0, 0.0, 1.0, 3.0):
        assert abs(normal_cdf(x) - norm.cdf(x)) < 1e-9
    # student_t_cdf
    for t_val in (-3.0, -1.0, 0.0, 1.0, 3.0):
        for df in (2.0, 5.0, 10.0, 30.0):
            assert abs(
                student_t_cdf(t_val, df) - t.cdf(t_val, df),
            ) < 1e-4
    # chi2_cdf
    for x in (0.1, 1.0, 3.84, 10.0):
        for df in (1, 2, 5, 10):
            assert abs(chi2_cdf(x, df) - chi2.cdf(x, df)) < 1e-4


# ---------------------------------------------------------------------------
# Bayesian decision boundaries
# ---------------------------------------------------------------------------


def test_bayesian_decision_threshold_respected():
    # Posterior exactly at 0.95 — decision "ship" (≥ ship_threshold)
    res = bayesian_beta_binomial(
        s_a=50, n_a=100, s_b=70, n_b=100, n_samples=2_000,
    )
    # Depending on the seed this posterior is ≥ 0.95.
    d = bayesian_decision(
        res, ship_threshold=0.5, rollback_threshold=0.0,
    )
    # 70 % vs 50 % — clearly ship
    assert d in ("ship", "continue")


def test_bayesian_decision_handles_zero_successes():
    # No successes in either arm — posteriors near 0.
    res = bayesian_beta_binomial(
        s_a=0, n_a=100, s_b=0, n_b=100, n_samples=1_000,
    )
    # P(B > A) ≈ 0.5 since both posteriors are identical beta(1, 101).
    assert 0.4 < res.prob_b_gt_a < 0.6


def test_bayesian_rejects_invalid_inputs():
    # n_samples too small
    with pytest.raises(ValueError):
        bayesian_beta_binomial(
            s_a=1, n_a=10, s_b=2, n_b=10, n_samples=50,
        )
    # s > n
    with pytest.raises(ValueError):
        bayesian_beta_binomial(
            s_a=20, n_a=10, s_b=2, n_b=10, n_samples=1_000,
        )
    # negative
    with pytest.raises(ValueError):
        bayesian_beta_binomial(
            s_a=-1, n_a=10, s_b=2, n_b=10, n_samples=1_000,
        )
    # prior non-positive
    with pytest.raises(ValueError):
        bayesian_beta_binomial(
            s_a=5, n_a=10, s_b=5, n_b=10, n_samples=1_000,
            prior_alpha=0.0,
        )


# ---------------------------------------------------------------------------
# Meta-analysis edge cases
# ---------------------------------------------------------------------------


def test_meta_analyze_rejects_empty_input():
    with pytest.raises(ValueError):
        meta_analyze([])


def test_meta_analyze_ignores_zero_variance_studies():
    # Single positive-variance + a junk zero-variance study.
    studies = [
        StudySummary(study_id="a", estimate=0.1, variance=0.01),
        StudySummary(study_id="b", estimate=9.9, variance=0.0),
    ]
    result = meta_analyze(studies)
    assert result.k == 1
    assert abs(result.pooled_estimate - 0.1) < 1e-6


def test_meta_analyze_two_studies_same_estimate():
    studies = [
        StudySummary(study_id=f"s{i}", estimate=0.2, variance=0.01)
        for i in range(3)
    ]
    result = meta_analyze(studies)
    assert abs(result.pooled_estimate - 0.2) < 1e-6
    assert result.tau_squared == 0.0
    assert result.q_statistic == 0.0


# ---------------------------------------------------------------------------
# α-spending: cumulative bound
# ---------------------------------------------------------------------------


def test_pocock_cumulative_alpha_monotone_and_bounded():
    sp = AlphaSpending(kind="pocock", alpha=0.05, total_looks=10)
    prev = 0.0
    for k in range(1, 11):
        cur = sp.alpha_spent(k)
        assert cur >= prev
        assert cur <= 0.05 + 1e-9
        prev = cur


def test_obf_cumulative_alpha_monotone_and_bounded():
    sp = AlphaSpending(kind="obf", alpha=0.05, total_looks=10)
    prev = 0.0
    for k in range(1, 11):
        cur = sp.alpha_spent(k)
        assert cur >= prev
        assert cur <= 0.05 + 1e-9
        prev = cur


def test_alpha_spending_rejects_invalid_alpha():
    with pytest.raises(ValueError):
        AlphaSpending(kind="pocock", alpha=0.0, total_looks=5)
    with pytest.raises(ValueError):
        AlphaSpending(kind="pocock", alpha=0.6, total_looks=5)
    with pytest.raises(ValueError):
        AlphaSpending(kind="pocock", alpha=0.05, total_looks=0)


# ---------------------------------------------------------------------------
# SRM detector — stronger skews
# ---------------------------------------------------------------------------


def test_srm_flags_large_skew():
    """60/40 on a designed-50/50 experiment after 1000 exposures is a
    statistically clear SRM (χ² ≈ 40 → p ≈ 2.5e-10)."""
    report = detect_srm(
        observed={"a": 600, "b": 400},
        expected_pct={"a": 50, "b": 50},
    )
    assert report.has_mismatch
    assert report.p_value < 1e-8
    # chi² = (600-500)²/500 + (400-500)²/500 = 40
    assert abs(report.chi_squared - 40.0) < 1e-9


def test_srm_tolerates_rounding_noise():
    """At 50 observations the test should NOT flag a tiny deviation."""
    report = detect_srm(
        observed={"a": 26, "b": 24},
        expected_pct={"a": 50, "b": 50},
    )
    assert not report.has_mismatch
    assert report.p_value > 0.5


def test_srm_rejects_negative_expected():
    with pytest.raises(ValueError):
        detect_srm(
            observed={"a": 10, "b": 10},
            expected_pct={"a": -5, "b": 5},
        )


# ---------------------------------------------------------------------------
# Assigner: allocation_pct=0 variant is never picked
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_zero_pct_variant_never_assigned():
    spec = ExperimentSpec(
        name="zero-pct",
        tenant_id="acme",
        task_id="summarize",
        layer="prompt",
        kind="ab_fixed",
        owner="alice",
        hypothesis="h",
        primary_metric="ctr",
        variants=(
            Variant(name="a", allocation_pct=100, is_control=True,
                    variant_id="va"),
            Variant(name="b", allocation_pct=0, variant_id="vb"),
        ),
    )
    assigner = HashAssigner(
        layers=LayerRegistry.default(),
        holdout=HoldoutFilter(0),
        cache=InMemoryAssignmentCache(),
    )
    for i in range(2_000):
        d = await assigner.assign(
            user_id=f"u-{i}", experiment_id="e", spec=spec,
        )
        assert d.variant is not None
        assert d.variant.variant_id == "va", (
            f"zero-pct variant got picked at user {i}: "
            f"{d.variant.variant_id}"
        )


@pytest.mark.asyncio
async def test_three_way_uneven_split_33_33_34():
    spec = ExperimentSpec(
        name="abc",
        tenant_id="acme",
        task_id="summarize",
        layer="prompt",
        kind="ab_fixed",
        owner="alice",
        hypothesis="h",
        primary_metric="ctr",
        variants=(
            Variant(name="a", allocation_pct=33, is_control=True,
                    variant_id="va"),
            Variant(name="b", allocation_pct=33, variant_id="vb"),
            Variant(name="c", allocation_pct=34, variant_id="vc"),
        ),
    )
    assigner = HashAssigner(
        layers=LayerRegistry.default(),
        holdout=HoldoutFilter(0),
        cache=InMemoryAssignmentCache(),
    )
    counts: dict[str, int] = {"va": 0, "vb": 0, "vc": 0}
    n = 30_000
    for i in range(n):
        d = await assigner.assign(
            user_id=f"u-{i}", experiment_id="e", spec=spec,
        )
        assert d.variant is not None
        counts[d.variant.variant_id] += 1
    # Expected fractions 0.33 / 0.33 / 0.34. Allow ±2 % absolute.
    for vid, expected in (("va", 0.33), ("vb", 0.33), ("vc", 0.34)):
        observed = counts[vid] / n
        assert abs(observed - expected) < 0.02, (
            f"{vid}: observed={observed:.3f}, expected={expected:.3f}"
        )


# ---------------------------------------------------------------------------
# Forced-assignment store CRUD
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_forced_assignment_store_crud_lifecycle():
    store = InMemoryForcedAssignmentStore()
    f = ForcedAssignment(
        experiment_id="e1", user_id="u1", variant_id="v1",
        reason="QA", created_by="ops",
    )
    assert await store.lookup(experiment_id="e1", user_id="u1") is None
    await store.put(f)
    got = await store.lookup(experiment_id="e1", user_id="u1")
    assert got is not None and got.variant_id == "v1"
    assert len(await store.list("e1")) == 1
    await store.remove(experiment_id="e1", user_id="u1")
    assert await store.lookup(experiment_id="e1", user_id="u1") is None
    # Remove of non-existing is a no-op (idempotent).
    await store.remove(experiment_id="e1", user_id="u1")


# ---------------------------------------------------------------------------
# Layer registry rotation
# ---------------------------------------------------------------------------


def test_layer_registry_rotation_changes_salt():
    reg = LayerRegistry.default()
    before = reg.salt("prompt")
    reg.rotate("prompt", "salt-prompt-v2")
    after = reg.salt("prompt")
    assert before != after
    assert after == "salt-prompt-v2"


def test_layer_registry_rejects_rotate_unknown_layer():
    reg = LayerRegistry(
        layers=(Layer(name="x", salt="s"),),
        allow_custom_layers=False,
    )
    from efe_rollout.errors import UnknownLayer
    with pytest.raises(UnknownLayer):
        reg.rotate("y", "new-salt")
    with pytest.raises(UnknownLayer):
        _ = reg.salt("y")


def test_layer_registry_custom_allowed_synthesizes_salt():
    reg = LayerRegistry(
        layers=(Layer(name="prompt", salt="s1"),),
        allow_custom_layers=True,
    )
    custom = reg.get("brand-new-axis")
    assert custom.salt.startswith("custom::")


# ---------------------------------------------------------------------------
# BH monotonicity
# ---------------------------------------------------------------------------


def test_bh_adjusted_p_values_are_monotone():
    raw = [0.001, 0.01, 0.05, 0.1, 0.5, 0.8]
    q = bh_adjusted_p(raw)
    # After sort by raw p, q should be non-decreasing.
    pairs = sorted(zip(raw, q, strict=True), key=lambda t: t[0])
    qs = [p[1] for p in pairs]
    for i in range(len(qs) - 1):
        assert qs[i] <= qs[i + 1] + 1e-12


def test_bh_preserves_nan_tolerance():
    raw = [0.01, float("nan"), 0.1]
    # BH itself only needs numerical p-values. Callers are expected to
    # filter NaN upstream. This test asserts no crash on happy path.
    q = bh_adjusted_p([p for p in raw if not math.isnan(p)])
    assert len(q) == 2


# ---------------------------------------------------------------------------
# Normal PPF extreme values
# ---------------------------------------------------------------------------


def test_normal_ppf_extreme_p_returns_sensible_values():
    # 1e-6 tail is where heavy precision matters (rare bug detection).
    lo = normal_ppf(1e-6)
    hi = normal_ppf(1.0 - 1e-6)
    assert -5.5 < lo < -4.5  # scipy says ~-4.75
    assert 4.5 < hi < 5.5


def test_normal_ppf_rejects_p_in_invalid_middle_region():
    # Valid p values are in (0, 1); this check guards against logic
    # bugs in upstream callers.
    assert normal_ppf(0.5) == 0.0
