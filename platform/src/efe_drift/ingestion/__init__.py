"""Ingestion plane : NATS consumer + buffer + pipeline + sink."""

from __future__ import annotations

from efe_drift.ingestion.buffer import EventBuffer
from efe_drift.ingestion.consumer import (
    ConsumerStats,
    IngestionWorker,
    InMemoryNatsSubscription,
    NatsConsumer,
    NatsMessage,
    NatsSubscription,
    decode_inference_event,
)
from efe_drift.ingestion.pipeline import IngestionPipeline
from efe_drift.ingestion.sink import InMemoryRawEventSink, RawEventSink

__all__ = [
    "ConsumerStats",
    "EventBuffer",
    "InMemoryNatsSubscription",
    "InMemoryRawEventSink",
    "IngestionPipeline",
    "IngestionWorker",
    "NatsConsumer",
    "NatsMessage",
    "NatsSubscription",
    "RawEventSink",
    "decode_inference_event",
]
