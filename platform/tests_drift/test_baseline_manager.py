"""BaselineManager — lifecycle, rotation, emergency retire."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone

import pytest

from efe_drift.baseline import (
    BaselineCandidate,
    BaselineManager,
    InMemoryBaselineStore,
)
from efe_drift.clock import FakeClock
from efe_drift.errors import BaselineSanityCheckFailed, ValidationError
from efe_drift.nats.publisher import InMemoryPublisher
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
    )


def _populated_candidate(entry: FeatureEntry, *, days: int = 10) -> BaselineCandidate:
    start = datetime(2026, 4, 1, tzinfo=timezone.utc)
    end = start + timedelta(days=days)
    cand = BaselineCandidate(
        entry=entry, window_start=start, window_end=end,
    )
    for i in range(12_000):
        day = i % days
        cand.ingest(FeatureSample(
            sample_id=f"s{i}",
            tenant_id=entry.tenant_id,
            task_id=entry.task_id,
            feature_id=entry.feature_id,
            feature_name=entry.name,
            observed_at=start + timedelta(days=day, hours=1),
            value=FeatureValue(
                kind="numeric",
                feature_name=entry.name,
                numeric_value=float(i % 100),
            ),
        ))
    return cand


async def _mk_manager() -> tuple[BaselineManager, FakeClock, InMemoryPublisher]:
    clock = FakeClock(epoch=datetime(2026, 4, 10, tzinfo=timezone.utc))
    store = InMemoryBaselineStore()
    pub = InMemoryPublisher()
    mgr = BaselineManager(
        store=store,
        signing_key=b"k" * 32,
        signing_key_id="kms/alias-1",
        default_overlap_days=3,
        clock=clock,
        publisher=pub,
    )
    return mgr, clock, pub


@pytest.mark.asyncio
async def test_full_promotion_lifecycle():
    mgr, clock, pub = await _mk_manager()
    entry = _numeric_entry()
    cand = _populated_candidate(entry)
    spec = cand.close(n_bins=10)
    baseline = await mgr.start_planned(spec=spec)
    # planned → accumulating → candidate
    baseline = await mgr.start_accumulating(baseline.baseline_id)
    updated, closed_spec, report = await mgr.close_candidate(
        baseline.baseline_id, candidate=cand,
    )
    assert updated.state == "candidate"
    assert report.ok
    # candidate → active
    active, plan, sealed = await mgr.promote(updated.baseline_id)
    assert active.state == "active"
    assert active.spec_hash is not None
    # No predecessor — first promotion.
    assert not plan.has_predecessor
    assert pub.messages(subject="efe.drift.baseline_promoted")


@pytest.mark.asyncio
async def test_rotation_promotes_new_and_retires_old():
    mgr, clock, pub = await _mk_manager()
    entry = _numeric_entry()

    async def _build_and_promote() -> str:
        cand = _populated_candidate(entry)
        spec = cand.close(n_bins=10)
        b = await mgr.start_planned(spec=spec)
        b = await mgr.start_accumulating(b.baseline_id)
        b, _, _ = await mgr.close_candidate(
            b.baseline_id, candidate=cand,
        )
        active, _, _ = await mgr.promote(b.baseline_id)
        return active.baseline_id

    first_id = await _build_and_promote()
    # Move the wall-clock forward before promoting the second so
    # promoted_at differs.
    clock.advance(10_000)
    second_id = await _build_and_promote()
    # First baseline should now be RETIRING.
    first = await mgr.get(first_id)
    assert first.state == "retiring"
    assert first.planned_end_at is not None
    # Second baseline is the sole ACTIVE.
    active = await mgr.active_for(
        tenant_id="acme", task_id="summarize",
        feature_name="prompt_length",
    )
    assert active is not None
    assert active.baseline_id == second_id


@pytest.mark.asyncio
async def test_tick_rotation_retires_after_overlap():
    mgr, clock, pub = await _mk_manager()
    entry = _numeric_entry()

    cand1 = _populated_candidate(entry)
    b1 = await mgr.start_planned(spec=cand1.close(n_bins=10))
    b1 = await mgr.start_accumulating(b1.baseline_id)
    b1, _, _ = await mgr.close_candidate(
        b1.baseline_id, candidate=cand1,
    )
    await mgr.promote(b1.baseline_id)

    clock.advance(3600)
    cand2 = _populated_candidate(entry)
    b2 = await mgr.start_planned(spec=cand2.close(n_bins=10))
    b2 = await mgr.start_accumulating(b2.baseline_id)
    b2, _, _ = await mgr.close_candidate(
        b2.baseline_id, candidate=cand2,
    )
    await mgr.promote(b2.baseline_id)

    # Advance past overlap (3 days default).
    clock.advance(int(timedelta(days=4).total_seconds()))
    retired = await mgr.tick_rotation()
    assert len(retired) == 1
    first = await mgr.get(b1.baseline_id)
    assert first.state == "retired"


@pytest.mark.asyncio
async def test_emergency_retire_sets_terminal_and_emits_event():
    mgr, clock, pub = await _mk_manager()
    entry = _numeric_entry()
    cand = _populated_candidate(entry)
    b = await mgr.start_planned(spec=cand.close(n_bins=10))
    b = await mgr.start_accumulating(b.baseline_id)
    b, _, _ = await mgr.close_candidate(
        b.baseline_id, candidate=cand,
    )
    await mgr.promote(b.baseline_id)

    retired = await mgr.emergency_retire(
        b.baseline_id, actor="ops:alice", reason="PII leak detected",
    )
    assert retired.state == "retired"
    assert retired.retired_at is not None
    assert pub.messages(subject="efe.drift.baseline_emergency_retired")


@pytest.mark.asyncio
async def test_promote_validates_sanity():
    mgr, clock, pub = await _mk_manager()
    entry = _numeric_entry()
    start = datetime(2026, 4, 1, tzinfo=timezone.utc)
    end = start + timedelta(days=10)
    cand = BaselineCandidate(
        entry=entry, window_start=start, window_end=end,
    )
    # Too few samples → will fail sanity
    for i in range(100):
        cand.ingest(FeatureSample(
            sample_id=f"s{i}",
            tenant_id=entry.tenant_id,
            task_id=entry.task_id,
            feature_id=entry.feature_id,
            feature_name=entry.name,
            observed_at=start + timedelta(hours=1),
            value=FeatureValue(
                kind="numeric",
                feature_name=entry.name,
                numeric_value=float(i),
            ),
        ))
    b = await mgr.start_planned(spec=cand.close(n_bins=10))
    b = await mgr.start_accumulating(b.baseline_id)
    with pytest.raises(BaselineSanityCheckFailed):
        await mgr.close_candidate(b.baseline_id, candidate=cand)


@pytest.mark.asyncio
async def test_verify_seal_accepts_untampered_baseline():
    mgr, clock, pub = await _mk_manager()
    entry = _numeric_entry()
    cand = _populated_candidate(entry)
    b = await mgr.start_planned(spec=cand.close(n_bins=10))
    b = await mgr.start_accumulating(b.baseline_id)
    b, _, _ = await mgr.close_candidate(
        b.baseline_id, candidate=cand,
    )
    active, _, _ = await mgr.promote(b.baseline_id)
    # Should not raise.
    await mgr.verify_seal(active.baseline_id)


@pytest.mark.asyncio
async def test_reject_candidate():
    mgr, clock, pub = await _mk_manager()
    entry = _numeric_entry()
    cand = _populated_candidate(entry)
    b = await mgr.start_planned(spec=cand.close(n_bins=10))
    b = await mgr.start_accumulating(b.baseline_id)
    b, _, _ = await mgr.close_candidate(
        b.baseline_id, candidate=cand,
    )
    rejected = await mgr.reject(b.baseline_id, actor="owner", reason="oops")
    assert rejected.state == "rejected"


@pytest.mark.asyncio
async def test_close_candidate_only_from_accumulating():
    mgr, clock, pub = await _mk_manager()
    entry = _numeric_entry()
    cand = _populated_candidate(entry)
    b = await mgr.start_planned(spec=cand.close(n_bins=10))
    with pytest.raises(ValidationError):
        await mgr.close_candidate(b.baseline_id, candidate=cand)
