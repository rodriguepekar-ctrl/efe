"""NATS envelope + HMAC helpers — identical semantics to M06.

Fix C5 — every HMAC input is prefixed with
``b"efe-inference/nats/v1\\x00"`` so a valid NATS signature cannot be
replayed as a SafetyAuditor record or as an M05 chain entry.

Fix M14 — ``verify_envelope`` clamps ``occurred_at`` to a skew window
so a replay of a stale envelope is refused.
"""

from __future__ import annotations

import base64
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import Any

from efe_inference.errors import InferenceError
from efe_inference.hashing import canonical_json_bytes, hmac_sign, hmac_verify

NATS_DOMAIN_TAG: bytes = b"efe-inference/nats/v1\x00"


class NatsMessageIntegrity(InferenceError):
    code = "nats_message_integrity"
    http_status = 401


@dataclass(frozen=True, slots=True)
class NatsEvent:
    event_type: str
    event_id: str
    occurred_at: datetime
    tenant_id: str
    payload: dict[str, Any] = field(default_factory=dict)
    schema_version: str = "1.0"

    def to_dict(self) -> dict[str, Any]:
        return {
            "schema_version": self.schema_version,
            "event_type": self.event_type,
            "event_id": self.event_id,
            "occurred_at": self.occurred_at.astimezone(
                timezone.utc,
            ).isoformat().replace("+00:00", "Z"),
            "tenant_id": self.tenant_id,
            "payload": dict(self.payload),
        }


def build_envelope(event: NatsEvent, *, key: bytes) -> dict[str, Any]:
    body = event.to_dict()
    mac = hmac_sign(NATS_DOMAIN_TAG + canonical_json_bytes(body), key)
    body["hmac"] = base64.b64encode(mac).decode("ascii")
    return body


DEFAULT_SKEW_S: int = 300  # 5-minute replay window


def verify_envelope(
    envelope: dict[str, Any],
    *,
    key: bytes,
    now: datetime | None = None,
    skew_s: int = DEFAULT_SKEW_S,
) -> NatsEvent:
    if "hmac" not in envelope:
        raise NatsMessageIntegrity("envelope missing hmac")
    body = dict(envelope)
    mac_b64 = body.pop("hmac")
    try:
        mac = base64.b64decode(mac_b64)
    except Exception as exc:
        raise NatsMessageIntegrity(f"bad base64: {exc}") from exc
    if not hmac_verify(NATS_DOMAIN_TAG + canonical_json_bytes(body), key, mac):
        raise NatsMessageIntegrity("hmac mismatch")
    try:
        occurred_at = datetime.fromisoformat(
            body["occurred_at"].replace("Z", "+00:00"),
        )
    except (KeyError, ValueError) as exc:
        raise NatsMessageIntegrity(f"bad occurred_at: {exc}") from exc
    # M14 — skew window
    now_utc = (now or datetime.now(timezone.utc)).astimezone(timezone.utc)
    drift = abs(now_utc - occurred_at.astimezone(timezone.utc))
    if drift > timedelta(seconds=skew_s):
        raise NatsMessageIntegrity(
            f"occurred_at outside skew window ({drift.total_seconds():.0f}s "
            f"vs {skew_s}s)",
        )
    return NatsEvent(
        schema_version=str(body.get("schema_version", "1.0")),
        event_type=str(body["event_type"]),
        event_id=str(body.get("event_id", str(uuid.uuid4()))),
        occurred_at=occurred_at,
        tenant_id=str(body.get("tenant_id", "")),
        payload=dict(body.get("payload", {})),
    )


__all__ = ["NatsEvent", "build_envelope", "verify_envelope"]
