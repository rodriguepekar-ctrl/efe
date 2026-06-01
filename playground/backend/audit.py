"""
EFE Playground — Audit log (HMAC-chained tamper-evident).

Mirrors D14-bis HMAC chain pattern :
  - Each event is hashed via HMAC-SHA-256 with a chain key
  - Each row's HMAC includes the previous row's HMAC (chain)
  - Tamper detection : any row alteration invalidates downstream HMACs

Simplified for playground :
  - In-memory ring buffer (1000 events max)
  - Single chain key (configurable via env)
  - JSON event payload

Production EFE uses Postgres + Object Lock 7-yr retention.
"""
from __future__ import annotations

import collections
import hashlib
import hmac
import json
import os
import time
import uuid
from dataclasses import dataclass, asdict
from typing import Optional


CHAIN_KEY_ENV = "EFE_PLAYGROUND_AUDIT_KEY"
DEFAULT_CHAIN_KEY = "playground-dev-audit-key-do-not-use-in-prod"
MAX_EVENTS = 1000


@dataclass
class AuditEvent:
    event_id: str
    timestamp: float
    tenant_id: str
    actor: str
    action: str           # "register", "classify", "delete", "evolve"
    resource: str         # catalog name or RPC method
    outcome: str          # "ok", "denied", "error"
    metadata: dict
    prev_hmac: str
    hmac: str

    def to_dict(self) -> dict:
        return asdict(self)


class AuditChain:
    def __init__(self, chain_key: Optional[str] = None) -> None:
        self.chain_key = (
            chain_key
            or os.environ.get(CHAIN_KEY_ENV, DEFAULT_CHAIN_KEY)
        ).encode("utf-8")
        self._events: collections.deque[AuditEvent] = collections.deque(
            maxlen=MAX_EVENTS
        )
        self._last_hmac = "0" * 64  # genesis

    def emit(
        self,
        *,
        tenant_id: str,
        actor: str,
        action: str,
        resource: str,
        outcome: str = "ok",
        metadata: Optional[dict] = None,
    ) -> AuditEvent:
        payload = {
            "tenant_id": tenant_id,
            "actor": actor,
            "action": action,
            "resource": resource,
            "outcome": outcome,
            "metadata": metadata or {},
        }
        canon = json.dumps(payload, sort_keys=True).encode("utf-8")
        # HMAC includes previous HMAC for chain integrity
        chained = self._last_hmac.encode("ascii") + canon
        new_hmac = hmac.new(self.chain_key, chained, hashlib.sha256).hexdigest()

        event = AuditEvent(
            event_id=str(uuid.uuid4()),
            timestamp=time.time(),
            tenant_id=tenant_id,
            actor=actor,
            action=action,
            resource=resource,
            outcome=outcome,
            metadata=metadata or {},
            prev_hmac=self._last_hmac,
            hmac=new_hmac,
        )
        self._events.append(event)
        self._last_hmac = new_hmac
        return event

    def list(
        self,
        *,
        tenant_id: Optional[str] = None,
        limit: int = 100,
    ) -> list[dict]:
        out = []
        for e in reversed(self._events):
            if tenant_id and e.tenant_id != tenant_id:
                continue
            out.append(e.to_dict())
            if len(out) >= limit:
                break
        return out

    def verify_integrity(self) -> dict:
        """Walk the chain + verify each row's HMAC.

        Returns : { ok: bool, count: int, broken_at: int | None }
        """
        prev_hmac = "0" * 64
        for i, e in enumerate(self._events):
            payload = {
                "tenant_id": e.tenant_id,
                "actor": e.actor,
                "action": e.action,
                "resource": e.resource,
                "outcome": e.outcome,
                "metadata": e.metadata,
            }
            canon = json.dumps(payload, sort_keys=True).encode("utf-8")
            chained = prev_hmac.encode("ascii") + canon
            expected = hmac.new(
                self.chain_key, chained, hashlib.sha256
            ).hexdigest()
            if expected != e.hmac:
                return {"ok": False, "count": len(self._events), "broken_at": i}
            if e.prev_hmac != prev_hmac:
                return {"ok": False, "count": len(self._events), "broken_at": i}
            prev_hmac = e.hmac
        return {"ok": True, "count": len(self._events), "broken_at": None}
