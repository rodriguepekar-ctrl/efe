"""End-to-end SDK wiring for L4 (evaluator) + L5 (router)."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone

import pytest

from efe_drift import (
    BaselineCandidate,
    ChannelRef,
    DriftClient,
    FeatureSpec,
    SeverityThresholds,
)
from efe_drift.alerting.channels.base import (
    DispatchResult,
    IncidentPayload,
)
from efe_drift.ids import new_uuid
from efe_drift.records import FeatureSample, FeatureSnapshot, FeatureValue


class _RecordChannel:
    name: str = "record"

    def __init__(self) -> None:
        self.payloads: list[IncidentPayload] = []

    async def dispatch(
        self, payload: IncidentPayload,
    ) -> DispatchResult:
        self.payloads.append(payload)
        return DispatchResult(
            ok=True, channel=self.name,
            delivered_at=datetime(2026, 4, 10, tzinfo=timezone.utc),
        )


@pytest.mark.asyncio
async def test_end_to_end_evaluate_and_route():
    """
    1. Register a numeric feature + promote a baseline.
    2. Push a minute snapshot that drifts heavily from baseline.
    3. Evaluate at 1h scale → at least one PSI result with score > 0.
    4. Pipe through the router with a rule whose hysteresis is 1 →
       incident fires on first result.
    5. Channel captures the payload.
    """
    client = DriftClient.in_memory()
    entry = await client.register_feature(FeatureSpec(
        tenant_id="acme", task_id="summarize",
        name="prompt_length_tokens",
        kind="numeric", extractor="token_length",
        binning_strategy="equifrequency",
    ))

    # Populated candidate → promote.
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
                kind="numeric", feature_name=entry.name,
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

    # Heavy-drift snapshot (all mass in last bin).
    now = datetime.now(tz=timezone.utc).replace(second=0, microsecond=0)
    snapshot = FeatureSnapshot(
        snapshot_id=new_uuid(),
        tenant_id=entry.tenant_id,
        task_id=entry.task_id,
        feature_id=entry.feature_id,
        feature_name=entry.name,
        feature_kind=entry.kind,
        bucket_kind="minute",
        bucket_start=now - timedelta(minutes=3),
        bucket_end=now - timedelta(minutes=2),
        sample_count=1000,
        histogram=(0, 0, 0, 0, 0, 0, 0, 0, 0, 1000),
    )
    client.push_snapshots((snapshot,))

    # Register an alert rule that fires immediately.
    channel = _RecordChannel()
    client.channel_registry.register(channel)
    await client.rule_store.put(await client.rule_store.create(
        tenant_id=entry.tenant_id, method="psi",
        thresholds=SeverityThresholds(
            warning=0.1, major=0.25, critical=0.4,
        ),
        channels=(ChannelRef(channel="record"),),
        consecutive_breaches=1,
    ) if False else await _create_rule(client, entry.tenant_id))

    # Evaluate + route.
    sweep = await client.evaluate_feature(entry, scales=("1h",))
    psi_results = sweep.by_method("psi")
    assert psi_results, "expected a PSI evaluation"
    assert psi_results[0].is_breach()

    decisions = await client.handle_drift_results(sweep.results)
    # At least one routing decision fired.
    fired = [d for d in decisions if d.fired]
    assert fired, "expected at least one firing decision"
    # Channel captured the payload.
    assert channel.payloads


async def _create_rule(client: DriftClient, tenant: str):
    return await client.rule_store.create(
        tenant_id=tenant,
        method="psi",
        thresholds=SeverityThresholds(
            warning=0.1, major=0.25, critical=0.4,
        ),
        channels=(ChannelRef(channel="record"),),
        consecutive_breaches=1,
    )


@pytest.mark.asyncio
async def test_cli_exposes_thresholds():
    from efe_drift.cli import main
    # No raise.
    assert main(["thresholds"]) == 0


@pytest.mark.asyncio
async def test_cli_version():
    from efe_drift.cli import main
    assert main(["version"]) == 0
