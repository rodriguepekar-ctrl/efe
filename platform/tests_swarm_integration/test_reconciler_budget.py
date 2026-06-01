"""Reconciler + budget tests."""

from __future__ import annotations

import pytest

from efe_swarm_integration.budget.estimator import HbmEstimator
from efe_swarm_integration.budget.hbm import HbmBudgeter
from efe_swarm_integration.budget.wallclock import compute_deadline
from efe_swarm_integration.config import MAX_HBM_BYTES
from efe_swarm_integration.errors import HbmBudgetExceeded
from efe_swarm_integration.reconciler.confidence import (
    compute_confidence,
    confidence_from_alignment,
)
from efe_swarm_integration.reconciler.decoder import (
    VerdictReconciler,
    reconcile,
)
from efe_swarm_integration.reconciler.nearest_anchor import nearest_anchor_label
from efe_swarm_integration.types import (
    ConeRef,
    DecodedOutput,
    ExecuteResponse,
    PopulationEntry,
    PopulationMix,
    SwarmSpec,
    TemplateCode,
    VerdictEnum,
)


# ---------------------------------------------------------------------------
# Confidence
# ---------------------------------------------------------------------------


def test_confidence_from_alignment_bounds() -> None:
    assert confidence_from_alignment(0.0, 1.0) == 1.0
    assert confidence_from_alignment(1.0, 1.0) == 0.0
    assert confidence_from_alignment(2.0, 1.0) == 0.0


def test_confidence_prefers_server() -> None:
    assert compute_confidence(
        alignment_delta=0.5, cone_radius=1.0, server_reported_confidence=0.92
    ) == pytest.approx(0.92)


# ---------------------------------------------------------------------------
# Nearest anchor
# ---------------------------------------------------------------------------


def test_nearest_anchor_label_basic() -> None:
    label, idx, d = nearest_anchor_label(
        [0.0, 0.0],
        anchors=[[1.0, 0.0], [0.0, 1.0], [10.0, 10.0]],
        labels=["a", "b", "c"],
    )
    assert label in {"a", "b"}
    assert idx in {0, 1}
    assert d < 2.0


# ---------------------------------------------------------------------------
# Reconcile
# ---------------------------------------------------------------------------


def _make_response(
    *,
    verdict: VerdictEnum = VerdictEnum.CONVERGED,
    delta: float = 0.2,
    confidence: float = 0.8,
    output_kind: str = "text",
    text: str = "fraud",
) -> ExecuteResponse:
    decoded = (
        DecodedOutput(kind="text", text=text)
        if output_kind == "text"
        else DecodedOutput(kind="raw_centroid", raw_centroid=(0.0, 0.5))
    )
    return ExecuteResponse(
        swarm_id="s",
        request_id="r",
        output=decoded,
        confidence=confidence,
        verdict=verdict,
        rounds_executed=3,
        total_ms=10.0,
        alignment_delta=delta,
    )


def test_reconcile_ok() -> None:
    resp = _make_response()
    bundle = reconcile(resp, cone_radius=0.35)
    assert bundle.is_converged()
    assert bundle.confidence > 0.5


def test_reconcile_divergence_no_raise_by_default() -> None:
    resp = _make_response(verdict=VerdictEnum.DIVERGENCE)
    bundle = reconcile(resp, cone_radius=0.35)
    assert bundle.verdict == VerdictEnum.DIVERGENCE


def test_reconciler_to_classify_text_path() -> None:
    bundle = reconcile(_make_response(), cone_radius=0.35)
    rec = VerdictReconciler()
    out = rec.to_classify(
        bundle, labels=["fraud", "billing"], anchors=None, cone_radius=0.35
    )
    assert out.label == "fraud"
    assert out.label_idx == 0
    assert not out.is_uncertain


def test_reconciler_to_classify_raw_centroid() -> None:
    resp = _make_response(output_kind="raw")
    bundle = reconcile(resp, cone_radius=0.35)
    rec = VerdictReconciler()
    out = rec.to_classify(
        bundle,
        labels=["a", "b"],
        anchors=[[0.0, 0.0], [1.0, 0.0]],
        cone_radius=0.35,
    )
    assert out.label in {"a", "b"}


def test_reconciler_uncertain_when_delta_near_radius() -> None:
    resp = _make_response(output_kind="raw", delta=0.33)
    bundle = reconcile(resp, cone_radius=0.35)
    rec = VerdictReconciler(uncertain_threshold_ratio=0.5)
    out = rec.to_classify(
        bundle,
        labels=["a"],
        anchors=[[0.0, 0.0]],
        cone_radius=0.35,
    )
    assert out.is_uncertain


# ---------------------------------------------------------------------------
# HBM budget
# ---------------------------------------------------------------------------


def _tiny_spec(dim: int = 4) -> SwarmSpec:
    return SwarmSpec(
        dim_latent=dim,
        problem_embedding=tuple([0.0] * dim),
        population=PopulationMix(entries=(PopulationEntry(TemplateCode.NaSem, 8),)),
        alignment_cone=None,
        alignment_cone_ref=ConeRef("t", "k", 1),
    )


def test_hbm_estimator_positive() -> None:
    est = HbmEstimator()
    val = est.estimate(_tiny_spec(), anchors_count=10)
    assert val > 0


def test_hbm_budgeter_raises_over_cap() -> None:
    est = HbmEstimator()
    bud = HbmBudgeter(estimator=est, hard_cap=1024)  # tiny cap
    with pytest.raises(HbmBudgetExceeded):
        bud.verify(_tiny_spec(dim=128), anchors_count=100)


def test_compute_deadline_adds_margin() -> None:
    from datetime import datetime, timezone, timedelta

    now = datetime(2026, 4, 23, tzinfo=timezone.utc)
    spec = _tiny_spec()
    dl = compute_deadline(spec, now=now)
    assert dl > now
    assert dl - now > timedelta(milliseconds=spec.budget.max_wall_clock_ms)
