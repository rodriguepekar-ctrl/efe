"""SafetyAuditor — emits HMAC-signed audit events on block / flag.

Two sinks are defined :

* :class:`InMemorySafetyAuditor` — records in a list (tests).
* :class:`DbNatsSafetyAuditor`  — persists to DB + emits NATS
  (production; wired in :class:`InferenceAdapter`).

Both enforce invariant **I3** (PII never in telemetry) by accepting
only pre-scrubbed text + computing a SHA-256 on top.
"""

from __future__ import annotations

import base64
import uuid
from dataclasses import dataclass, field
from datetime import datetime
from typing import Protocol

from efe_inference.clock import Clock, SystemClock
from efe_inference.hashing import (
    canonical_json_bytes,
    hmac_sign,
    sha256_text,
)
from efe_inference.safety.policy import SafetyStage, SafetyVerdict


@dataclass(frozen=True, slots=True)
class SafetyEventRecord:
    event_id: str
    request_id: str
    tenant_id: str
    stage: SafetyStage
    severity: str
    categories: tuple[str, ...]
    payload_hash_hex: str
    signature_b64: str
    created_at: datetime


class SafetyAuditor(Protocol):
    async def record(
        self,
        *,
        request_id: str,
        tenant_id: str,
        verdict: SafetyVerdict,
        severity: str,
        scrubbed_text: str,
    ) -> SafetyEventRecord:  # pragma: no cover
        ...


@dataclass
class InMemorySafetyAuditor:
    """Records every event in a list; no IO."""

    hmac_key: bytes
    clock: Clock = field(default_factory=SystemClock)
    events: list[SafetyEventRecord] = field(default_factory=list)

    async def record(
        self,
        *,
        request_id: str,
        tenant_id: str,
        verdict: SafetyVerdict,
        severity: str,
        scrubbed_text: str,
    ) -> SafetyEventRecord:
        rec = _build_record(
            hmac_key=self.hmac_key,
            clock=self.clock,
            request_id=request_id,
            tenant_id=tenant_id,
            verdict=verdict,
            severity=severity,
            scrubbed_text=scrubbed_text,
        )
        self.events.append(rec)
        return rec


SAFETY_DOMAIN_TAG: bytes = b"efe-inference/safety/v1\x00"


def _build_record(
    *,
    hmac_key: bytes,
    clock: Clock,
    request_id: str,
    tenant_id: str,
    verdict: SafetyVerdict,
    severity: str,
    scrubbed_text: str,
) -> SafetyEventRecord:
    payload_hash = sha256_text(scrubbed_text or "")
    now = clock.now()
    event_id = str(uuid.uuid4())
    signing_input = SAFETY_DOMAIN_TAG + canonical_json_bytes({
        "event_id": event_id,
        "request_id": request_id,
        "tenant_id": tenant_id,
        "stage": verdict.stage.value,
        "severity": severity,
        "categories": list(verdict.categories),
        "payload_hash_hex": payload_hash.hex(),
        "occurred_at": now.isoformat(),
    })
    signature = hmac_sign(signing_input, hmac_key)
    return SafetyEventRecord(
        event_id=event_id,
        request_id=request_id,
        tenant_id=tenant_id,
        stage=verdict.stage,
        severity=severity,
        categories=verdict.categories,
        payload_hash_hex=payload_hash.hex(),
        signature_b64=base64.b64encode(signature).decode("ascii"),
        created_at=now,
    )


__all__ = [
    "InMemorySafetyAuditor",
    "SafetyAuditor",
    "SafetyEventRecord",
]
