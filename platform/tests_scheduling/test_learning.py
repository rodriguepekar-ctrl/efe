"""Tests for learning decisions and continuous-learning pipeline."""

from __future__ import annotations

import asyncio
from datetime import datetime, timedelta, timezone

import pytest

from efe_scheduling.clock import FakeClock
from efe_scheduling.learning import (
    BaselineRefreshInputs,
    BaselineRefreshOutcome,
    CanaryReport,
    ContinuousLearningPipeline,
    LearningConfig,
    PromotionInputs,
    RetrainInputs,
    RollbackInputs,
    ValidationReport,
    ValidationThresholds,
    build_dataset_manifest,
    decide_baseline_refresh,
    decide_promotion,
    decide_retrain,
    decide_rollback,
    validate_report,
)
from efe_scheduling.learning.canary import CanaryStatus
from efe_scheduling.learning.curator import CuratorInputs
from efe_scheduling.learning.promotion import PromotionOutcome
from efe_scheduling.learning.retrain import RetrainOutcome
from efe_scheduling.learning.rollback import RollbackOutcome
from efe_scheduling.triggers.drift import DriftEvent, DriftSeverity


# ---------------------------------------------------------------------------
# Baseline refresh
# ---------------------------------------------------------------------------


def test_baseline_refresh_ok_when_eligible() -> None:
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    inp = BaselineRefreshInputs(
        feature_id="f",
        feature_active=True,
        last_refresh_at=now - timedelta(days=2),
        now=now,
        sample_count_last_window=5_000,
        min_samples=1_000,
        variance_ratio=0.1,
        max_variance_ratio=0.25,
    )
    d = decide_baseline_refresh(inp, min_interval_h=12)
    assert d.outcome == BaselineRefreshOutcome.refresh


def test_baseline_refresh_skipped_if_inactive() -> None:
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    d = decide_baseline_refresh(
        BaselineRefreshInputs(
            feature_id="f",
            feature_active=False,
            last_refresh_at=None,
            now=now,
            sample_count_last_window=0,
            min_samples=1_000,
            variance_ratio=0.0,
            max_variance_ratio=0.25,
        )
    )
    assert d.outcome == BaselineRefreshOutcome.skip_inactive


def test_baseline_refresh_skipped_if_recently_refreshed() -> None:
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    d = decide_baseline_refresh(
        BaselineRefreshInputs(
            feature_id="f",
            feature_active=True,
            last_refresh_at=now - timedelta(minutes=30),
            now=now,
            sample_count_last_window=5_000,
            min_samples=1_000,
            variance_ratio=0.1,
            max_variance_ratio=0.25,
        ),
        min_interval_h=6,
    )
    assert d.outcome == BaselineRefreshOutcome.skip_recently_refreshed


def test_baseline_refresh_sanity_failure() -> None:
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    d = decide_baseline_refresh(
        BaselineRefreshInputs(
            feature_id="f",
            feature_active=True,
            last_refresh_at=None,
            now=now,
            sample_count_last_window=10,
            min_samples=1_000,
            variance_ratio=0.0,
            max_variance_ratio=0.25,
        )
    )
    assert d.outcome == BaselineRefreshOutcome.skip_sanity_failed


# ---------------------------------------------------------------------------
# Retrain decision
# ---------------------------------------------------------------------------


def test_retrain_freezes_during_cooldown() -> None:
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    d = decide_retrain(
        RetrainInputs(
            now=now,
            last_retrain_at=now,
            cool_down_until=now + timedelta(hours=1),
            max_drift_severity="critical",
            critical_feature_count=5,
        )
    )
    assert d.outcome == RetrainOutcome.freeze


def test_retrain_triggers_on_critical() -> None:
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    d = decide_retrain(
        RetrainInputs(
            now=now,
            last_retrain_at=now,
            cool_down_until=None,
            max_drift_severity="critical",
            critical_feature_count=5,
            min_critical_features=3,
        )
    )
    assert d.outcome == RetrainOutcome.retrain


def test_retrain_hold_otherwise() -> None:
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    d = decide_retrain(
        RetrainInputs(
            now=now,
            last_retrain_at=now - timedelta(days=3),
            cool_down_until=None,
            max_drift_severity="warning",
            critical_feature_count=0,
            overdue_threshold_d=30,
        )
    )
    assert d.outcome == RetrainOutcome.hold


def test_retrain_triggers_when_overdue() -> None:
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    d = decide_retrain(
        RetrainInputs(
            now=now,
            last_retrain_at=now - timedelta(days=30),
            cool_down_until=None,
            max_drift_severity="warning",
            critical_feature_count=0,
            overdue_threshold_d=14,
        )
    )
    assert d.outcome == RetrainOutcome.retrain


# ---------------------------------------------------------------------------
# Validation
# ---------------------------------------------------------------------------


def test_validation_accepts_good_metrics() -> None:
    r = validate_report(
        {"perplexity": 50, "rouge_l": 0.3, "bleu": 0.2, "exact_match": 0.25},
        ValidationThresholds(),
    )
    assert r.ready


def test_validation_rejects_bad_metrics() -> None:
    r = validate_report(
        {"perplexity": 500, "rouge_l": 0.1, "bleu": 0.1, "exact_match": 0.1},
        ValidationThresholds(),
    )
    assert not r.ready
    assert len(r.reasons) >= 3


# ---------------------------------------------------------------------------
# Promotion
# ---------------------------------------------------------------------------


def test_promotion_rollback_on_error_rate() -> None:
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    canary = CanaryReport(
        status=CanaryStatus.observing,
        started_at=now,
        observed_at=now + timedelta(hours=3),
        latency_p99_delta_ms=0.0,
        error_rate_delta=0.05,
        drift_score_delta=0.0,
        quality_metric_delta=0.0,
    )
    val = validate_report({"perplexity": 10, "rouge_l": 0.5, "bleu": 0.4, "exact_match": 0.4}, ValidationThresholds())
    d = decide_promotion(
        PromotionInputs(
            canary=canary,
            validation=val,
            gate_major_failures=0,
            canary_started_at=canary.started_at,
            now=now + timedelta(hours=3),
            max_error_rate_delta=0.01,
        )
    )
    assert d.outcome == PromotionOutcome.rollback


def test_promotion_promotes_when_healthy() -> None:
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    canary = CanaryReport(
        status=CanaryStatus.healthy,
        started_at=now,
        observed_at=now + timedelta(hours=3),
    )
    val = validate_report({"perplexity": 10, "rouge_l": 0.5, "bleu": 0.4, "exact_match": 0.4}, ValidationThresholds())
    d = decide_promotion(
        PromotionInputs(
            canary=canary,
            validation=val,
            canary_started_at=canary.started_at,
            now=now + timedelta(hours=3),
            canary_observation_h=2,
        )
    )
    assert d.outcome == PromotionOutcome.promote


def test_promotion_holds_if_gate_failed() -> None:
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    canary = CanaryReport(
        status=CanaryStatus.healthy,
        started_at=now,
        observed_at=now + timedelta(hours=3),
    )
    val = validate_report({"perplexity": 10, "rouge_l": 0.5, "bleu": 0.4, "exact_match": 0.4}, ValidationThresholds())
    d = decide_promotion(
        PromotionInputs(
            canary=canary,
            validation=val,
            gate_major_failures=1,
            canary_started_at=canary.started_at,
            now=now + timedelta(hours=3),
        )
    )
    assert d.outcome == PromotionOutcome.hold


# ---------------------------------------------------------------------------
# Rollback
# ---------------------------------------------------------------------------


def test_rollback_on_latency_spike() -> None:
    d = decide_rollback(
        RollbackInputs(
            tenant_id="t1",
            task_id=None,
            error_rate_current=0.01,
            error_rate_baseline=0.01,
            latency_p99_current_ms=800,
            latency_p99_baseline_ms=500,
            drift_score_current=0.0,
            drift_score_baseline=0.0,
        )
    )
    assert d.outcome == RollbackOutcome.rollback


def test_rollback_healthy_holds() -> None:
    d = decide_rollback(
        RollbackInputs(
            tenant_id="t1",
            task_id=None,
            error_rate_current=0.01,
            error_rate_baseline=0.01,
            latency_p99_current_ms=500,
            latency_p99_baseline_ms=500,
            drift_score_current=0.0,
            drift_score_baseline=0.0,
        )
    )
    assert d.outcome == RollbackOutcome.hold


# ---------------------------------------------------------------------------
# Curator
# ---------------------------------------------------------------------------


def test_curator_produces_signed_manifest() -> None:
    now = datetime(2026, 4, 21, tzinfo=timezone.utc)
    out = build_dataset_manifest(
        CuratorInputs(
            tenant_id="t1",
            task_id=None,
            training_cycle_id="cycle-1",
            window_end=now,
            window_days=30,
        )
    )
    assert len(out.manifest.seal) == 32
    assert len(out.manifest.seed) == 32


# ---------------------------------------------------------------------------
# Pipeline
# ---------------------------------------------------------------------------


def test_pipeline_on_drift_critical_enqueues_retrain() -> None:
    fc = FakeClock(datetime(2026, 4, 21, tzinfo=timezone.utc))
    pipe = ContinuousLearningPipeline(
        cfg=LearningConfig(retrain_min_critical_features=1),
        clock=fc,
    )
    ev = DriftEvent(
        result_id="r",
        tenant_id="t1",
        task_id=None,
        feature_name="latency_ms",
        method="psi",
        score=0.6,
        severity=DriftSeverity.critical,
        observed_at=fc.now(),
        window_start=fc.now() - timedelta(hours=1),
        window_end=fc.now(),
    )
    jobs = asyncio.run(pipe.on_drift(ev))
    kinds = [j.kind.value for j in jobs]
    # Expect at least a retrain_decision (and possibly a baseline_refresh too).
    assert "retrain_decision" in kinds
