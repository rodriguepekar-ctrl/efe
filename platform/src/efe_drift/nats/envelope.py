"""NATS envelope — HMAC-signed, skew-protected.

Identical structural contract to :mod:`efe_rollout.nats.envelope` but
tagged with :data:`efe_drift.hashing.DOMAIN_TAG_NATS` so a signature
legitimate on a rollout event cannot be replayed on a drift event.

Envelope shape ::

    {
        "schema_version": "1.0",
        "event_type":     "efe.drift.detected",
        "event_id":       "<uuid>",
        "occurred_at":    "<ISO-8601 UTC>",
        "tenant_id":      "<tenant>",
        "payload":        { ... },
        "hmac":           "<hex>"
    }

The HMAC is computed over canonical JSON of every field *except*
``hmac``, with :data:`DOMAIN_TAG_NATS` prepended.

Missing-field handling uses the same "validate shape before HMAC"
pattern as :mod:`efe_rollout` — a ``KeyError`` is never allowed to
escape.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any

from efe_drift.clock import Clock, SystemClock
from efe_drift.errors import NatsEnvelopeStale, NatsMessageIntegrity
from efe_drift.hashing import (
    DOMAIN_TAG_NATS,
    canonical_json_bytes,
    hmac_sign,
    hmac_verify,
)
from efe_drift.ids import new_uuid


@dataclass(frozen=True, slots=True)
class NatsEnvelope:
    schema_version: str
    event_type: str
    event_id: str
    occurred_at: datetime
    tenant_id: str
    payload: dict[str, Any]
    hmac: bytes

    def to_dict(self) -> dict[str, Any]:
        return {
            "schema_version": self.schema_version,
            "event_type": self.event_type,
            "event_id": self.event_id,
            "occurred_at": self.occurred_at.isoformat(),
            "tenant_id": self.tenant_id,
            "payload": self.payload,
            "hmac": self.hmac.hex(),
        }


def _body_for_hmac(envelope: dict[str, Any]) -> bytes:
    body = {k: v for k, v in envelope.items() if k != "hmac"}
    return canonical_json_bytes(body)


class EnvelopeSigner:
    """Produce signed envelopes on the outbound edge."""

    __slots__ = ("_clock", "_key", "_schema_version")

    def __init__(
        self,
        *,
        hmac_key: bytes,
        schema_version: str = "1.0",
        clock: Clock | None = None,
    ) -> None:
        if len(hmac_key) < 16:
            raise ValueError("HMAC key too short")
        self._key = bytes(hmac_key)
        self._schema_version = schema_version
        self._clock = clock or SystemClock()

    def sign(
        self,
        *,
        event_type: str,
        tenant_id: str,
        payload: dict[str, Any],
        event_id: str | None = None,
        occurred_at: datetime | None = None,
    ) -> NatsEnvelope:
        eid = event_id or new_uuid()
        when = occurred_at or self._clock.now()
        if when.tzinfo is None:
            when = when.replace(tzinfo=timezone.utc)
        body = {
            "schema_version": self._schema_version,
            "event_type": event_type,
            "event_id": eid,
            "occurred_at": when.isoformat(),
            "tenant_id": tenant_id,
            "payload": payload,
        }
        sig = hmac_sign(
            _body_for_hmac(body), self._key, domain_tag=DOMAIN_TAG_NATS,
        )
        return NatsEnvelope(
            schema_version=self._schema_version,
            event_type=event_type,
            event_id=eid,
            occurred_at=when,
            tenant_id=tenant_id,
            payload=payload,
            hmac=sig,
        )


class EnvelopeVerifier:
    """Verify incoming envelopes + freshness against a skew window."""

    _REQUIRED_FIELDS: tuple[str, ...] = (
        "schema_version",
        "event_type",
        "event_id",
        "occurred_at",
        "tenant_id",
        "hmac",
    )

    __slots__ = ("_clock", "_key", "_skew_window_s")

    def __init__(
        self,
        *,
        hmac_key: bytes,
        skew_window_s: int = 300,
        clock: Clock | None = None,
    ) -> None:
        if len(hmac_key) < 16:
            raise ValueError("HMAC key too short")
        self._key = bytes(hmac_key)
        self._skew_window_s = int(skew_window_s)
        self._clock = clock or SystemClock()

    def verify(self, envelope: dict[str, Any]) -> NatsEnvelope:
        # Shape validation — before HMAC check.
        missing = [
            f for f in self._REQUIRED_FIELDS if f not in envelope
        ]
        if missing:
            raise NatsMessageIntegrity(
                f"envelope missing required fields: {sorted(missing)!r}",
            )
        sig_hex = envelope["hmac"]
        if not isinstance(sig_hex, str):
            raise NatsMessageIntegrity("hmac field must be a hex string")
        try:
            sig = bytes.fromhex(sig_hex)
        except ValueError as exc:
            raise NatsMessageIntegrity(
                f"hmac not hex: {exc}",
            ) from exc
        if not hmac_verify(
            _body_for_hmac(envelope), self._key, sig,
            domain_tag=DOMAIN_TAG_NATS,
        ):
            raise NatsMessageIntegrity("envelope HMAC does not verify")
        try:
            occurred_at = datetime.fromisoformat(
                str(envelope["occurred_at"]),
            )
        except (TypeError, ValueError) as exc:
            raise NatsMessageIntegrity(
                f"invalid occurred_at: {exc}",
            ) from exc
        if occurred_at.tzinfo is None:
            occurred_at = occurred_at.replace(tzinfo=timezone.utc)
        delta = abs(
            (self._clock.now() - occurred_at).total_seconds(),
        )
        if delta > self._skew_window_s:
            raise NatsEnvelopeStale(
                f"envelope skew {delta:.1f}s exceeds window "
                f"{self._skew_window_s}s",
                detail={"event_id": envelope.get("event_id")},
            )
        return NatsEnvelope(
            schema_version=str(envelope["schema_version"]),
            event_type=str(envelope["event_type"]),
            event_id=str(envelope["event_id"]),
            occurred_at=occurred_at,
            tenant_id=str(envelope["tenant_id"]),
            payload=dict(envelope.get("payload") or {}),
            hmac=sig,
        )


__all__ = [
    "EnvelopeSigner",
    "EnvelopeVerifier",
    "NatsEnvelope",
]
