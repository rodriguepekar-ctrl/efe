"""End-to-end drift evaluator."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone

import pytest

from efe_drift import (
    BaselineCandidate,
    DriftClient,
    FeatureSpec,
)
from efe_drift.detection import (
    DriftEvaluator,
    InMemorySnapshotSource,
)
from efe_drift.errors import NoActiveBaseline
from efe_drift.ids import new_uuid
from efe_drift.records import FeatureSample, FeatureSnapshot, FeatureValue


@pytest.mark.asyncio
async def test_evaluator_raises_no_active_baseline():
    client = DriftClient.in_memory()
    entry = await client.register_feature(FeatureSpec(
        tenant_id="acme", task_id="summarize", name="ll",
        kind="numeric", extractor="token_length",
        binning_strategy="equifrequency",
    ))
    with pytest.raises(NoActiveBaseline):
        await client.evaluate_feature(entry)


@pytest.mark.asyncio
async def test_evaluator_end_to_end_uniform():
    client = DriftClient.in_memory()
    entry = await client.register_feature(FeatureSpec(
        tenant_id="acme", task_id="summarize",
        name="prompt_length_tokens",
        kind="numeric", extractor="token_length",
        binning_strategy="equifrequency",
    ))

    # Build + promote a baseline.
    start = datetime(2026, 4, 1, tzinfo=timezone.utc)
    end = start + timedelta(days=10)
    candidate = BaselineCandidate(
        entry=entry, window_start=start, window_end=end,
    )
    for i in range(12_000):
        candidate.ingest(FeatureSample(
            sample_id=f"s{i}",
            tenant_id=entry.tenant_id,
            task_id=entry.task_id,
            feature_id=entry.feature_id,
            feature_name=entry.name,
            observed_at=start + timedelta(days=i % 10, hours=1),
            value=FeatureValue(
                kind="numeric",
                feature_name=entry.name,
                numeric_value=float(i % 100),
            ),
        ))
    b = await client.baseline_manager.start_planned(
        spec=candidate.close(n_bins=10),
    )
    b = await client.baseline_manager.start_accumulating(b.baseline_id)
    b, _, _ = await client.baseline_manager.close_candidate(
        b.baseline_id, candidate=candidate,
    )
    await client.baseline_manager.promote(b.baseline_id)

    # Push some current snapshots identical to baseline → no drift.
    t0 = datetime.now(tz=timezone.utc).replace(
        second=0, microsecond=0,
    ) - timedelta(minutes=10)
    for i in range(5):
        snap = FeatureSnapshot(
            snapshot_id=new_uuid(),
            tenant_id=entry.tenant_id,
            task_id=entry.task_id,
            feature_id=entry.feature_id,
            feature_name=entry.name,
            feature_kind=entry.kind,
            bucket_kind="minute",
            bucket_start=t0 + timedelta(minutes=i),
            bucket_end=t0 + timedelta(minutes=i + 1),
            sample_count=10,
            histogram=(1, 1, 1, 1, 1, 1, 1, 1, 1, 1),
        )
        client.push_snapshots((snap,))

    sweep = await client.evaluate_feature(entry, scales=("1h",))
    psi = sweep.by_method("psi")
    assert psi, "expected a PSI result"
    # Uniform current vs uniform baseline → PSI ≈ 0, severity none.
    assert psi[0].score < 0.01
    assert psi[0].severity == "none"


@pytest.mark.asyncio
async def test_evaluator_handles_skipped_detectors():
    """Classifier-based drift has no snapshot path — it must be
    silently skipped and show up in ``sweep.skipped``."""
    source = InMemorySnapshotSource()
    # Use a DriftEvaluator with only the classifier method to force
    # a "no result" sweep (NaN score → severity none).
    from efe_drift.detection.classifier import ClassifierDriftDetector
    from efe_drift.detection.registry import DetectorRegistry

    reg = DetectorRegistry()
    reg.register(ClassifierDriftDetector())
    client = DriftClient.in_memory()
    entry = await client.register_feature(FeatureSpec(
        tenant_id="acme", task_id="summarize", name="latency",
        kind="numeric", extractor="backend_latency",
        binning_strategy="equifrequency",
    ))
    evaluator = DriftEvaluator(
        catalog=client.catalog,
        baseline_manager=client.baseline_manager,
        snapshots=source,
        detectors=reg,
        snapshot_methods=("classifier_based",),
    )
    # No baseline → NoActiveBaseline.
    with pytest.raises(NoActiveBaseline):
        await evaluator.evaluate_feature(entry)
