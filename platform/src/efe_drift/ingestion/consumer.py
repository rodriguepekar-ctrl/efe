"""NATS consumer for drift ingestion.

The production path pulls events from ``efe.inference.completed`` and
``efe.rollout.outcome`` via JetStream. This module abstracts that over
a minimal :class:`NatsSubscription` Protocol so tests can replay a
scripted stream of envelopes.

Responsibilities :

* Verify the envelope (HMAC + domain tag + skew window) via
  :class:`EnvelopeVerifier`.
* Decode the envelope payload into an :class:`InferenceEvent`.
* Push to :class:`EventBuffer` — :class:`BufferFull` surfaces as
  backpressure.
* Track :attr:`backpressure_since_s` for the observability plane to
  alert on.

The consumer does **not** run the extractor pipeline inline. It
enqueues ; a separate worker task drains the buffer and calls
:class:`IngestionPipeline`. This split means slow extractors (e.g.
SBERT) cannot pause the NATS consumer, which would cause ack timeouts.
"""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Protocol

from efe_drift.clock import Clock, SystemClock
from efe_drift.errors import BufferFull, NatsMessageIntegrity
from efe_drift.ingestion.buffer import EventBuffer
from efe_drift.ingestion.pipeline import IngestionPipeline
from efe_drift.nats.envelope import EnvelopeVerifier
from efe_drift.records import InferenceEvent

# ---------------------------------------------------------------------------
# Subscription Protocol
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class NatsMessage:
    subject: str
    data: dict[str, Any]            # decoded envelope dict
    ack: Callable[[], Awaitable[None]]
    nack: Callable[[], Awaitable[None]]


class NatsSubscription(Protocol):
    async def __anext__(self) -> NatsMessage:  # pragma: no cover
        ...

    async def aclose(self) -> None:  # pragma: no cover
        ...


# ---------------------------------------------------------------------------
# In-memory subscription — test fixture
# ---------------------------------------------------------------------------


class InMemoryNatsSubscription:
    """Scripted subscription used in tests.

    Messages are enqueued via :meth:`push` ; the consumer iterates
    with ``__anext__``. :meth:`close` is called to end the stream.
    """

    __slots__ = ("_acked", "_closed", "_nacked", "_queue")

    def __init__(self) -> None:
        self._queue: asyncio.Queue[NatsMessage | None] = asyncio.Queue()
        self._closed = False
        self._acked: list[str] = []
        self._nacked: list[str] = []

    async def push(self, subject: str, payload: dict[str, Any]) -> None:
        """Enqueue a pre-parsed envelope dict for the consumer."""
        sub_id = payload.get("event_id") or subject
        sub_ref = sub_id

        async def _ack() -> None:
            self._acked.append(sub_ref)

        async def _nack() -> None:
            self._nacked.append(sub_ref)

        msg = NatsMessage(
            subject=subject, data=payload, ack=_ack, nack=_nack,
        )
        await self._queue.put(msg)

    async def close(self) -> None:
        await self._queue.put(None)
        self._closed = True

    async def aclose(self) -> None:
        await self.close()

    def __aiter__(self) -> InMemoryNatsSubscription:
        return self

    async def __anext__(self) -> NatsMessage:
        msg = await self._queue.get()
        if msg is None:
            raise StopAsyncIteration
        return msg

    @property
    def acked(self) -> tuple[str, ...]:
        return tuple(self._acked)

    @property
    def nacked(self) -> tuple[str, ...]:
        return tuple(self._nacked)


# ---------------------------------------------------------------------------
# Envelope → InferenceEvent decoder
# ---------------------------------------------------------------------------


def decode_inference_event(envelope_payload: dict[str, Any]) -> InferenceEvent:
    """Decode the inner ``payload`` dict of a verified envelope.

    Required fields :

    * ``event_id``, ``tenant_id``, ``task_id``, ``user_id``,
      ``observed_at``.

    Optional fields map 1-to-1 onto :class:`InferenceEvent`.
    """
    required = ("event_id", "tenant_id", "task_id", "user_id", "observed_at")
    missing = [f for f in required if f not in envelope_payload]
    if missing:
        raise NatsMessageIntegrity(
            f"inference event missing required fields: {missing!r}",
            detail={"missing": missing},
        )
    observed_raw = envelope_payload["observed_at"]
    try:
        observed_at = datetime.fromisoformat(str(observed_raw))
    except (TypeError, ValueError) as exc:
        raise NatsMessageIntegrity(
            f"invalid observed_at: {exc}",
        ) from exc
    if observed_at.tzinfo is None:
        observed_at = observed_at.replace(tzinfo=timezone.utc)
    covariates_raw = envelope_payload.get("covariates") or {}
    if not isinstance(covariates_raw, dict):
        raise NatsMessageIntegrity("covariates must be an object")
    return InferenceEvent(
        event_id=str(envelope_payload["event_id"]),
        tenant_id=str(envelope_payload["tenant_id"]),
        task_id=str(envelope_payload["task_id"]),
        user_id=str(envelope_payload["user_id"]),
        observed_at=observed_at,
        request_id=_opt_str(envelope_payload.get("request_id")),
        prompt_token_count=_opt_int(
            envelope_payload.get("prompt_token_count"),
        ),
        response_token_count=_opt_int(
            envelope_payload.get("response_token_count"),
        ),
        latency_ms=_opt_float(envelope_payload.get("latency_ms")),
        cost_usd=_opt_float(envelope_payload.get("cost_usd")),
        model_id=_opt_str(envelope_payload.get("model_id")),
        backend_kind=_opt_str(envelope_payload.get("backend_kind")),
        temperature=_opt_float(envelope_payload.get("temperature")),
        top_p=_opt_float(envelope_payload.get("top_p")),
        prompt_text=_opt_str(envelope_payload.get("prompt_text")),
        response_text=_opt_str(envelope_payload.get("response_text")),
        covariates=dict(covariates_raw),
    )


def _opt_str(v: Any) -> str | None:
    return None if v is None else str(v)


def _opt_int(v: Any) -> int | None:
    if v is None:
        return None
    try:
        return int(v)
    except (TypeError, ValueError):
        return None


def _opt_float(v: Any) -> float | None:
    if v is None:
        return None
    try:
        return float(v)
    except (TypeError, ValueError):
        return None


# ---------------------------------------------------------------------------
# NatsConsumer — the main loop
# ---------------------------------------------------------------------------


class NatsConsumer:
    """Drives the pull-based consumption loop.

    Usage ::

        consumer = NatsConsumer(
            subscription=sub,
            verifier=verifier,
            buffer=buffer,
        )
        await consumer.run()   # blocks until subscription is closed

    The consumer acks the envelope **after** it lands in the buffer.
    If the envelope fails integrity / decode, we nack and let
    JetStream retry up to ``max_deliver`` times before dead-letter.
    """

    __slots__ = (
        "_buffer",
        "_clock",
        "_last_backpressure_s",
        "_subscription",
        "_verifier",
        "stats",
    )

    def __init__(
        self,
        *,
        subscription: NatsSubscription,
        verifier: EnvelopeVerifier,
        buffer: EventBuffer[InferenceEvent],
        clock: Clock | None = None,
    ) -> None:
        self._subscription = subscription
        self._verifier = verifier
        self._buffer = buffer
        self._clock = clock or SystemClock()
        self._last_backpressure_s: float | None = None
        self.stats = ConsumerStats()

    @property
    def backpressure_since_s(self) -> float | None:
        """Monotonic timestamp of the first BufferFull; None otherwise."""
        return self._last_backpressure_s

    async def run(self) -> None:
        try:
            async for msg in self._subscription:            # type: ignore[attr-defined]
                await self._process_one(msg)
        finally:
            await self._subscription.aclose()

    async def process_one(self, msg: NatsMessage) -> None:
        """Exposed for tests — run the consumer logic on a single msg."""
        await self._process_one(msg)

    async def _process_one(self, msg: NatsMessage) -> None:
        self.stats.received += 1
        # 1. Verify envelope.
        try:
            envelope = self._verifier.verify(msg.data)
        except NatsMessageIntegrity:
            self.stats.integrity_failures += 1
            await msg.nack()
            return

        # 2. Decode inner payload.
        try:
            event = decode_inference_event(envelope.payload)
        except NatsMessageIntegrity:
            self.stats.decode_failures += 1
            await msg.nack()
            return

        # 3. Enqueue — backpressure surface.
        try:
            await self._buffer.put(event)
        except BufferFull:
            self.stats.backpressure_events += 1
            if self._last_backpressure_s is None:
                self._last_backpressure_s = self._clock.now_monotonic_s()
            await msg.nack()
            return

        # 4. Ack.
        self.stats.accepted += 1
        self._last_backpressure_s = None
        await msg.ack()


@dataclass(slots=True)
class ConsumerStats:
    """Plain-old counters used by the observability plane."""

    received: int = 0
    accepted: int = 0
    integrity_failures: int = 0
    decode_failures: int = 0
    backpressure_events: int = 0


# ---------------------------------------------------------------------------
# Worker loop that drains the buffer into the pipeline
# ---------------------------------------------------------------------------


class IngestionWorker:
    """Simple async worker that drains the buffer into the pipeline.

    Separated from :class:`NatsConsumer` so heavy extractors (SBERT)
    cannot back-pressure NATS ack.
    """

    __slots__ = ("_batch_size", "_buffer", "_pipeline", "_poll_interval_s")

    def __init__(
        self,
        *,
        buffer: EventBuffer[InferenceEvent],
        pipeline: IngestionPipeline,
        batch_size: int = 256,
        poll_interval_s: float = 0.5,
    ) -> None:
        self._buffer = buffer
        self._pipeline = pipeline
        self._batch_size = batch_size
        self._poll_interval_s = poll_interval_s

    async def drain_once(self) -> int:
        """Drain up to ``batch_size`` events; returns the count
        processed. Useful for deterministic tests."""
        batch = await self._buffer.drain_batch(self._batch_size)
        if not batch:
            return 0
        await self._pipeline.process_batch(batch)
        return len(batch)

    async def run_until_empty(self) -> int:
        """Keep draining until the buffer is empty — test helper."""
        total = 0
        while True:
            n = await self.drain_once()
            if n == 0:
                return total
            total += n

    async def run_forever(self, *, stop_event: asyncio.Event) -> int:
        """Production loop — polls the buffer until ``stop_event`` is
        set. Returns the total number of events processed."""
        total = 0
        while not stop_event.is_set():
            n = await self.drain_once()
            if n == 0:
                got_item = await self._buffer.wait_non_empty(
                    timeout=self._poll_interval_s,
                )
                if not got_item and stop_event.is_set():
                    return total
                continue
            total += n
        return total


__all__ = [
    "ConsumerStats",
    "InMemoryNatsSubscription",
    "IngestionWorker",
    "NatsConsumer",
    "NatsMessage",
    "NatsSubscription",
    "decode_inference_event",
]
