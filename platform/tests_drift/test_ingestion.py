"""Ingestion — buffer + pipeline + consumer."""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from efe_drift.catalog import FeatureCatalog, FeatureSpec
from efe_drift.clock import FakeClock
from efe_drift.errors import BufferFull, NatsMessageIntegrity
from efe_drift.extraction import build_default_registry
from efe_drift.ingestion import (
    EventBuffer,
    InMemoryNatsSubscription,
    InMemoryRawEventSink,
    IngestionPipeline,
    IngestionWorker,
    NatsConsumer,
    decode_inference_event,
)
from efe_drift.nats import EnvelopeSigner, EnvelopeVerifier
from efe_drift.records import InferenceEvent


# ---------------------------------------------------------------------------
# Buffer
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_event_buffer_put_and_drain():
    buffer = EventBuffer[int](capacity=3)
    await buffer.put(1)
    await buffer.put(2)
    assert buffer.size() == 2
    batch = await buffer.drain_batch(10)
    assert batch == (1, 2)
    assert buffer.is_empty()


@pytest.mark.asyncio
async def test_event_buffer_full_raises():
    buffer = EventBuffer[int](capacity=2)
    await buffer.put(1)
    await buffer.put(2)
    with pytest.raises(BufferFull):
        await buffer.put(3)


def test_event_buffer_rejects_zero_capacity():
    with pytest.raises(ValueError):
        EventBuffer[int](capacity=0)


# ---------------------------------------------------------------------------
# IngestionPipeline
# ---------------------------------------------------------------------------


async def _build_pipeline() -> tuple[IngestionPipeline, InMemoryRawEventSink]:
    extractors = build_default_registry()
    catalog = FeatureCatalog(extractor_lookup=extractors)
    await catalog.register(FeatureSpec(
        tenant_id="acme", task_id="summarize",
        name="prompt_length_tokens",
        kind="numeric", extractor="token_length",
        binning_strategy="equifrequency",
    ))
    await catalog.register(FeatureSpec(
        tenant_id="acme", task_id="summarize",
        name="language",
        kind="categorical", extractor="lang_detect",
        binning_strategy="none",
    ))
    sink = InMemoryRawEventSink()
    pipeline = IngestionPipeline(
        catalog=catalog,
        extractors=extractors,
        sinks=[sink],
        user_id_salt=b"\x01" * 32,
    )
    return pipeline, sink


@pytest.mark.asyncio
async def test_pipeline_processes_event_into_samples():
    pipeline, sink = await _build_pipeline()
    event = InferenceEvent(
        event_id="e",
        tenant_id="acme",
        task_id="summarize",
        user_id="u1",
        observed_at=datetime(2026, 4, 20, tzinfo=timezone.utc),
        prompt_token_count=100,
        prompt_text="Hello there friend",
    )
    samples = await pipeline.process(event)
    assert len(samples) == 2
    assert {s.feature_name for s in samples} == {
        "prompt_length_tokens", "language",
    }
    assert len(sink) == 2


@pytest.mark.asyncio
async def test_pipeline_skips_features_returning_none():
    pipeline, sink = await _build_pipeline()
    # language extractor requires prompt_text — omit it here
    event = InferenceEvent(
        event_id="e",
        tenant_id="acme",
        task_id="summarize",
        user_id="u1",
        observed_at=datetime(2026, 4, 20, tzinfo=timezone.utc),
        prompt_token_count=100,
    )
    samples = await pipeline.process(event)
    feature_names = {s.feature_name for s in samples}
    assert feature_names == {"prompt_length_tokens"}


@pytest.mark.asyncio
async def test_pipeline_hashes_user_id():
    pipeline, sink = await _build_pipeline()
    event = InferenceEvent(
        event_id="e",
        tenant_id="acme",
        task_id="summarize",
        user_id="raw-user-id-42",
        observed_at=datetime(2026, 4, 20, tzinfo=timezone.utc),
        prompt_token_count=10,
    )
    samples = await pipeline.process(event)
    assert samples
    for s in samples:
        assert s.user_id_hash is not None
        assert s.user_id_hash != "raw-user-id-42"
        assert len(s.user_id_hash) == 64   # SHA-256 hex


@pytest.mark.asyncio
async def test_pipeline_cross_tenant_isolation():
    """An event for an unregistered tenant produces zero samples."""
    pipeline, sink = await _build_pipeline()
    event = InferenceEvent(
        event_id="e",
        tenant_id="other-tenant",
        task_id="summarize",
        user_id="u",
        observed_at=datetime(2026, 4, 20, tzinfo=timezone.utc),
        prompt_token_count=10,
        prompt_text="Hello",
    )
    samples = await pipeline.process(event)
    assert samples == ()
    assert len(sink) == 0


# ---------------------------------------------------------------------------
# NatsConsumer
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_nats_consumer_happy_path():
    key = b"k" * 32
    clock = FakeClock()
    signer = EnvelopeSigner(hmac_key=key, clock=clock)
    verifier = EnvelopeVerifier(
        hmac_key=key, skew_window_s=300, clock=clock,
    )
    sub = InMemoryNatsSubscription()
    buffer = EventBuffer[InferenceEvent](capacity=10)
    consumer = NatsConsumer(
        subscription=sub, verifier=verifier, buffer=buffer,
        clock=clock,
    )
    env = signer.sign(
        event_type="efe.inference.completed",
        tenant_id="acme",
        payload={
            "event_id": "e-1",
            "tenant_id": "acme",
            "task_id": "summarize",
            "user_id": "u",
            "observed_at": clock.now().isoformat(),
            "prompt_token_count": 10,
        },
    )
    await sub.push("efe.inference.completed", env.to_dict())
    await sub.close()
    await consumer.run()
    assert consumer.stats.received == 1
    assert consumer.stats.accepted == 1
    # Exactly one ack, matching the envelope's own event_id (not the
    # inner payload's — they're intentionally distinct).
    assert len(sub.acked) == 1
    assert sub.nacked == ()


@pytest.mark.asyncio
async def test_nats_consumer_rejects_tampered_envelope():
    key = b"k" * 32
    clock = FakeClock()
    signer = EnvelopeSigner(hmac_key=key, clock=clock)
    verifier = EnvelopeVerifier(
        hmac_key=key, skew_window_s=300, clock=clock,
    )
    sub = InMemoryNatsSubscription()
    buffer = EventBuffer[InferenceEvent](capacity=10)
    consumer = NatsConsumer(
        subscription=sub, verifier=verifier, buffer=buffer, clock=clock,
    )
    env = signer.sign(
        event_type="efe.inference.completed",
        tenant_id="acme",
        payload={
            "event_id": "e-1",
            "tenant_id": "acme",
            "task_id": "summarize",
            "user_id": "u",
            "observed_at": clock.now().isoformat(),
        },
    )
    d = env.to_dict()
    d["payload"]["tenant_id"] = "other"   # tampered
    await sub.push("x", d)
    await sub.close()
    await consumer.run()
    assert consumer.stats.integrity_failures == 1
    assert consumer.stats.accepted == 0


@pytest.mark.asyncio
async def test_nats_consumer_backpressure_nacks_when_buffer_full():
    key = b"k" * 32
    clock = FakeClock()
    signer = EnvelopeSigner(hmac_key=key, clock=clock)
    verifier = EnvelopeVerifier(
        hmac_key=key, skew_window_s=300, clock=clock,
    )
    sub = InMemoryNatsSubscription()
    buffer = EventBuffer[InferenceEvent](capacity=1)
    consumer = NatsConsumer(
        subscription=sub, verifier=verifier, buffer=buffer, clock=clock,
    )
    for i in range(3):
        env = signer.sign(
            event_type="x", tenant_id="acme",
            payload={
                "event_id": f"e-{i}",
                "tenant_id": "acme",
                "task_id": "summarize",
                "user_id": "u",
                "observed_at": clock.now().isoformat(),
            },
        )
        await sub.push("x", env.to_dict())
    await sub.close()
    await consumer.run()
    # First goes through; next two hit backpressure.
    assert consumer.stats.accepted == 1
    assert consumer.stats.backpressure_events == 2
    assert consumer.backpressure_since_s is not None


# ---------------------------------------------------------------------------
# decode_inference_event
# ---------------------------------------------------------------------------


def test_decode_inference_event_missing_field_raises():
    with pytest.raises(NatsMessageIntegrity):
        decode_inference_event({})


def test_decode_inference_event_bad_observed_at():
    with pytest.raises(NatsMessageIntegrity):
        decode_inference_event({
            "event_id": "e",
            "tenant_id": "a",
            "task_id": "b",
            "user_id": "u",
            "observed_at": "not-a-date",
        })


# ---------------------------------------------------------------------------
# IngestionWorker
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_ingestion_worker_drains_buffer():
    pipeline, sink = await _build_pipeline()
    buffer = EventBuffer[InferenceEvent](capacity=10)
    for i in range(3):
        await buffer.put(InferenceEvent(
            event_id=f"e-{i}",
            tenant_id="acme",
            task_id="summarize",
            user_id="u",
            observed_at=datetime(2026, 4, 20, tzinfo=timezone.utc),
            prompt_token_count=10 + i,
            prompt_text="Hello there friend",
        ))
    worker = IngestionWorker(buffer=buffer, pipeline=pipeline, batch_size=5)
    n = await worker.run_until_empty()
    assert n == 3
    assert len(sink) >= 3
