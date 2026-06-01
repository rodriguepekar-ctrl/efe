"""Baseline candidate — accumulation + close + validator."""

from __future__ import annotations

import math
from datetime import datetime, timedelta, timezone

import pytest

from efe_drift.baseline import BaselineCandidate, BaselineValidator
from efe_drift.errors import (
    BaselineSanityCheckFailed,
    EmptySampleSet,
    ExtractorTypeMismatch,
)
from efe_drift.records import FeatureEntry, FeatureSample, FeatureValue


def _numeric_entry() -> FeatureEntry:
    return FeatureEntry(
        feature_id="f-1",
        tenant_id="acme",
        task_id="summarize",
        name="prompt_length",
        version=1,
        kind="numeric",
        extractor="token_length",
        binning_strategy="equifrequency",
        retention_days=90,
        created_at=datetime(2026, 4, 1, tzinfo=timezone.utc),
    )


def _categorical_entry() -> FeatureEntry:
    return FeatureEntry(
        feature_id="f-2",
        tenant_id="acme",
        task_id="summarize",
        name="language",
        version=1,
        kind="categorical",
        extractor="lang_detect",
        binning_strategy="none",
        retention_days=90,
    )


def _emb_entry() -> FeatureEntry:
    return FeatureEntry(
        feature_id="f-3",
        tenant_id="acme",
        task_id="summarize",
        name="prompt_embedding",
        version=1,
        kind="embedding",
        extractor="deterministic_hash_embedding",
        binning_strategy="none",
        retention_days=90,
    )


def _numeric_sample(entry: FeatureEntry, v: float, when: datetime) -> FeatureSample:
    return FeatureSample(
        sample_id="s",
        tenant_id=entry.tenant_id,
        task_id=entry.task_id,
        feature_id=entry.feature_id,
        feature_name=entry.name,
        observed_at=when,
        value=FeatureValue(
            kind="numeric",
            feature_name=entry.name,
            numeric_value=v,
        ),
    )


def _cat_sample(entry: FeatureEntry, v: str, when: datetime) -> FeatureSample:
    return FeatureSample(
        sample_id="s",
        tenant_id=entry.tenant_id,
        task_id=entry.task_id,
        feature_id=entry.feature_id,
        feature_name=entry.name,
        observed_at=when,
        value=FeatureValue(
            kind="categorical",
            feature_name=entry.name,
            categorical_value=v,
        ),
    )


def _emb_sample(
    entry: FeatureEntry, vec: tuple[float, ...], when: datetime,
) -> FeatureSample:
    return FeatureSample(
        sample_id="s",
        tenant_id=entry.tenant_id,
        task_id=entry.task_id,
        feature_id=entry.feature_id,
        feature_name=entry.name,
        observed_at=when,
        value=FeatureValue(
            kind="embedding",
            feature_name=entry.name,
            embedding=vec,
        ),
    )


# ---------------------------------------------------------------------------
# Accumulation
# ---------------------------------------------------------------------------


def test_candidate_numeric_accumulation_and_close():
    entry = _numeric_entry()
    start = datetime(2026, 4, 1, tzinfo=timezone.utc)
    end = start + timedelta(days=7)
    candidate = BaselineCandidate(
        entry=entry, window_start=start, window_end=end,
    )
    # 100 uniform samples across 7 days
    for i in range(100):
        t = start + timedelta(days=(i % 7), hours=1)
        candidate.ingest(_numeric_sample(entry, float(i), t))
    assert candidate.sample_count == 100
    assert candidate.days_covered == 7
    spec = candidate.close(n_bins=10)
    assert spec.bins is not None
    assert len(spec.bins) == 11
    assert spec.sample_count == 100
    assert spec.stats["mean"] == pytest.approx(49.5)


def test_candidate_ignores_samples_outside_window():
    entry = _numeric_entry()
    start = datetime(2026, 4, 1, tzinfo=timezone.utc)
    end = start + timedelta(days=1)
    candidate = BaselineCandidate(
        entry=entry, window_start=start, window_end=end,
    )
    # Inside window
    candidate.ingest(_numeric_sample(entry, 1.0, start + timedelta(hours=1)))
    # Outside window — should be silently ignored
    candidate.ingest(_numeric_sample(entry, 2.0, end + timedelta(hours=1)))
    assert candidate.sample_count == 1


def test_candidate_rejects_wrong_kind():
    entry = _numeric_entry()
    start = datetime(2026, 4, 1, tzinfo=timezone.utc)
    end = start + timedelta(days=1)
    candidate = BaselineCandidate(
        entry=entry, window_start=start, window_end=end,
    )
    with pytest.raises(ExtractorTypeMismatch):
        candidate.ingest(_cat_sample(entry, "fr", start + timedelta(hours=1)))


def test_candidate_categorical_close():
    entry = _categorical_entry()
    start = datetime(2026, 4, 1, tzinfo=timezone.utc)
    end = start + timedelta(days=7)
    candidate = BaselineCandidate(
        entry=entry, window_start=start, window_end=end,
    )
    for v in ["fr", "fr", "fr", "en", "en", "es"]:
        candidate.ingest(_cat_sample(
            entry, v, start + timedelta(hours=1),
        ))
    spec = candidate.close(top_k=2)
    # Top-K = 2 → ``fr``, ``en`` kept. OTHER stripped from frozen list.
    assert spec.categorical_categories == ("en", "fr")


def test_candidate_embedding_close():
    entry = _emb_entry()
    start = datetime(2026, 4, 1, tzinfo=timezone.utc)
    end = start + timedelta(days=7)
    candidate = BaselineCandidate(
        entry=entry, window_start=start, window_end=end,
    )
    for i in range(10):
        candidate.ingest(_emb_sample(
            entry, (float(i), float(-i), 0.0, 0.0),
            start + timedelta(hours=1),
        ))
    spec = candidate.close()
    assert spec.centroid is not None
    assert len(spec.centroid) == 4
    assert math.isclose(spec.centroid[0], 4.5, abs_tol=1e-9)
    assert math.isclose(spec.centroid[1], -4.5, abs_tol=1e-9)


def test_candidate_close_without_samples_raises():
    entry = _numeric_entry()
    start = datetime(2026, 4, 1, tzinfo=timezone.utc)
    end = start + timedelta(days=1)
    candidate = BaselineCandidate(
        entry=entry, window_start=start, window_end=end,
    )
    with pytest.raises(EmptySampleSet):
        candidate.close()


# ---------------------------------------------------------------------------
# Validator
# ---------------------------------------------------------------------------


def test_validator_flags_insufficient_samples():
    entry = _numeric_entry()
    start = datetime(2026, 4, 1, tzinfo=timezone.utc)
    end = start + timedelta(days=7)
    candidate = BaselineCandidate(
        entry=entry, window_start=start, window_end=end,
    )
    # only 5 samples → well below default min_sample_count=10_000
    for i in range(5):
        candidate.ingest(
            _numeric_sample(entry, float(i), start + timedelta(hours=1)),
        )
    spec = candidate.close(n_bins=3)
    report = BaselineValidator().validate(spec)
    assert not report.ok
    codes = {issue.code for issue in report.blockers}
    assert "insufficient_samples" in codes


def test_validator_flags_short_window():
    entry = _numeric_entry()
    start = datetime(2026, 4, 1, tzinfo=timezone.utc)
    end = start + timedelta(days=2)
    candidate = BaselineCandidate(
        entry=entry, window_start=start, window_end=end,
    )
    for i in range(12_000):
        candidate.ingest(
            _numeric_sample(entry, float(i % 1000),
                            start + timedelta(hours=1)),
        )
    spec = candidate.close(n_bins=10)
    report = BaselineValidator(
        min_sample_count=10_000, min_days=7,
    ).validate(spec)
    assert not report.ok
    codes = {issue.code for issue in report.blockers}
    assert "short_window_blocker" in codes


def test_validator_detects_zero_variance():
    entry = _numeric_entry()
    start = datetime(2026, 4, 1, tzinfo=timezone.utc)
    end = start + timedelta(days=7)
    candidate = BaselineCandidate(
        entry=entry, window_start=start, window_end=end,
    )
    for _ in range(12_000):
        candidate.ingest(
            _numeric_sample(entry, 42.0, start + timedelta(hours=1)),
        )
    spec = candidate.close(n_bins=10)
    report = BaselineValidator().validate(spec)
    codes = {issue.code for issue in report.blockers}
    assert "zero_variance" in codes


def test_validator_strict_raises_on_blocker():
    entry = _numeric_entry()
    start = datetime(2026, 4, 1, tzinfo=timezone.utc)
    end = start + timedelta(days=1)
    candidate = BaselineCandidate(
        entry=entry, window_start=start, window_end=end,
    )
    for i in range(100):
        candidate.ingest(
            _numeric_sample(entry, float(i),
                            start + timedelta(hours=1)),
        )
    spec = candidate.close(n_bins=10)
    with pytest.raises(BaselineSanityCheckFailed):
        BaselineValidator().validate_strict(spec)


def test_validator_categorical_single_category_rejected():
    entry = _categorical_entry()
    start = datetime(2026, 4, 1, tzinfo=timezone.utc)
    end = start + timedelta(days=10)
    candidate = BaselineCandidate(
        entry=entry, window_start=start, window_end=end,
    )
    for _ in range(12_000):
        candidate.ingest(_cat_sample(
            entry, "fr", start + timedelta(hours=1),
        ))
    spec = candidate.close(top_k=5)
    report = BaselineValidator().validate(spec)
    codes = {issue.code for issue in report.blockers}
    assert "single_category" in codes


def test_validator_embedding_zero_centroid_rejected():
    entry = _emb_entry()
    start = datetime(2026, 4, 1, tzinfo=timezone.utc)
    end = start + timedelta(days=10)
    candidate = BaselineCandidate(
        entry=entry, window_start=start, window_end=end,
    )
    for _ in range(12_000):
        candidate.ingest(_emb_sample(
            entry, (0.0, 0.0, 0.0, 0.0),
            start + timedelta(hours=1),
        ))
    spec = candidate.close()
    report = BaselineValidator().validate(spec)
    codes = {issue.code for issue in report.blockers}
    assert "zero_centroid" in codes
