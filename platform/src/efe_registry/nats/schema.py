"""NATS event envelope + HMAC helpers.

Every inter-module message is wrapped::

    {
      "schema_version": "1.0",
      "event_type": "efe.registry.artifact.signed",
      "event_id": "018e5f3a-...",
      "occurred_at": "2026-04-20T10:12:34.567Z",
      "tenant_id": "acme",
      "payload": { ... },
      "hmac": "<base64 HMAC-SHA256>"
    }

The HMAC is computed over the canonical JSON bytes of every field
except ``hmac`` itself. Consumers verify before processing.
"""

from __future__ import annotations

import base64
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any

from efe_registry.errors import NatsMessageIntegrity
from efe_registry.hashing import canonical_json_bytes, hmac_sign, hmac_verify


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


def build_envelope(
    event: NatsEvent, *, key: bytes,
) -> dict[str, Any]:
    body = event.to_dict()
    mac = hmac_sign(canonical_json_bytes(body), key)
    body["hmac"] = base64.b64encode(mac).decode("ascii")
    return body


def verify_envelope(envelope: dict[str, Any], *, key: bytes) -> NatsEvent:
    if "hmac" not in envelope:
        raise NatsMessageIntegrity("envelope missing hmac")
    body = dict(envelope)
    mac_b64 = body.pop("hmac")
    try:
        mac = base64.b64decode(mac_b64)
    except Exception as exc:
        raise NatsMessageIntegrity(f"bad base64 hmac: {exc}") from exc
    if not hmac_verify(canonical_json_bytes(body), key, mac):
        raise NatsMessageIntegrity("hmac mismatch on NATS envelope")
    try:
        occurred_at = datetime.fromisoformat(
            body["occurred_at"].replace("Z", "+00:00"),
        )
    except (KeyError, ValueError) as exc:
        raise NatsMessageIntegrity(
            f"bad occurred_at: {exc}",
        ) from exc
    return NatsEvent(
        schema_version=str(body.get("schema_version", "1.0")),
        event_type=str(body["event_type"]),
        event_id=str(body.get("event_id", str(uuid.uuid4()))),
        occurred_at=occurred_at,
        tenant_id=str(body.get("tenant_id", "")),
        payload=dict(body.get("payload", {})),
    )


__all__ = ["NatsEvent", "build_envelope", "verify_envelope"]
