"""End-to-end DriftClient SDK integration test."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone

import pytest

from efe_drift import DriftClient, FeatureSpec
from efe_drift.records import InferenceEvent


@pytest.mark.asyncio
async def test_sdk_end_to_end_sample_and_rollup():
    client = DriftClient.in_memory()
    await client.register_feature(FeatureSpec(
        tenant_id="acme",
        task_id="summarize",
        name="prompt_length_tokens",
        kind="numeric",
        extractor="token_length",
        binning_strategy="equifrequency",
    ))
    await client.register_feature(FeatureSpec(
        tenant_id="acme",
        task_id="summarize",
        name="language",
        kind="categorical",
        extractor="lang_detect",
        binning_strategy="none",
    ))
    t0 = datetime(2026, 4, 20, 10, 0, 0, tzinfo=timezone.utc)
    for i in range(30):
        await client.ingest_event(InferenceEvent(
            event_id=f"e-{i}",
            tenant_id="acme",
            task_id="summarize",
            user_id=f"u-{i}",
            observed_at=t0 + timedelta(seconds=i),
            prompt_token_count=100 + i,
            prompt_text="Hello how are you today",
        ))
    samples = client.sink.all()  # type: ignore[attr-defined]
    assert len(samples) == 60   # 30 events × 2 features
    snapshots = client.rollup_samples_to_snapshots(
        samples, bucket_kind="minute",
    )
    assert len(snapshots) >= 2


@pytest.mark.asyncio
async def test_sdk_promotes_baseline_via_manager():
    from efe_drift import BaselineCandidate
    client = DriftClient.in_memory()
    entry = await client.register_feature(FeatureSpec(
        tenant_id="acme",
        task_id="summarize",
        name="prompt_length_tokens",
        kind="numeric",
        extractor="token_length",
        binning_strategy="equifrequency",
    ))
    # Build a populated candidate + promote.
    start = datetime(2026, 4, 1, tzinfo=timezone.utc)
    end = start + timedelta(days=10)
    cand = BaselineCandidate(
        entry=entry, window_start=start, window_end=end,
    )
    for i in range(12_000):
        cand.ingest(__import__(
            "efe_drift.records", fromlist=["FeatureSample"],
        ).FeatureSample(
            sample_id=f"s{i}",
            tenant_id=entry.tenant_id,
            task_id=entry.task_id,
            feature_id=entry.feature_id,
            feature_name=entry.name,
            observed_at=start + timedelta(
                days=i % 10, hours=1,
            ),
            value=__import__(
                "efe_drift.records", fromlist=["FeatureValue"],
            ).FeatureValue(
                kind="numeric",
                feature_name=entry.name,
                numeric_value=float(i % 100),
            ),
        ))
    mgr = client.baseline_manager
    b = await mgr.start_planned(spec=cand.close(n_bins=10))
    b = await mgr.start_accumulating(b.baseline_id)
    b, _, _ = await mgr.close_candidate(
        b.baseline_id, candidate=cand,
    )
    active, _, _ = await mgr.promote(b.baseline_id)
    assert active.state == "active"
