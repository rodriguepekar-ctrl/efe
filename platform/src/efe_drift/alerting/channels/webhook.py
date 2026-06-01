"""Generic HMAC-signed webhook channel.

The channel POSTs the :class:`IncidentPayload` as JSON to a
configurable URL. Each request carries an ``X-EFE-Drift-Signature``
header whose value is ``hex(HMAC-SHA256(secret, body))``. Callers
verify using the same key.

Retries : exponential backoff with 3 attempts (1 s, 2 s, 4 s) on
transient 5xx / network failures. We stop on 4xx.
"""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field
from typing import Any

from efe_drift.alerting.channels.base import (
    DispatchResult,
    IncidentPayload,
)
from efe_drift.clock import Clock, SystemClock
from efe_drift.hashing import (
    DOMAIN_TAG_NATS,
    canonical_json_bytes,
    hmac_sign,
)

WebhookPoster = Callable[
    [str, dict[str, Any], dict[str, str]],
    Awaitable[tuple[int, str]],
]


@dataclass(frozen=True, slots=True)
class WebhookChannel:
    url: str
    hmac_key: bytes
    http_poster: WebhookPoster
    max_retries: int = 3
    backoff_s: float = 1.0
    clock: Clock = field(default_factory=SystemClock)
    _name: str = "webhook"

    def __post_init__(self) -> None:
        if len(self.hmac_key) < 16:
            raise ValueError(
                "webhook hmac_key too short (need ≥ 16 bytes)",
            )

    @property
    def name(self) -> str:
        return self._name

    async def dispatch(self, payload: IncidentPayload) -> DispatchResult:
        body = payload.to_dict()
        raw = canonical_json_bytes(body)
        signature = hmac_sign(
            raw, self.hmac_key, domain_tag=DOMAIN_TAG_NATS,
        ).hex()
        headers = {
            "Content-Type": "application/json",
            "X-EFE-Drift-Signature": signature,
            "X-EFE-Drift-Incident": payload.incident_id,
            "X-EFE-Drift-Severity": payload.severity,
        }
        last_err: str | None = None
        for attempt in range(1, self.max_retries + 1):
            try:
                status, text = await self.http_poster(
                    self.url, body, headers,
                )
            except Exception as exc:
                last_err = f"{type(exc).__name__}: {exc}"
                await self._sleep(attempt)
                continue
            if 200 <= status < 300:
                return DispatchResult(
                    ok=True, channel=self._name,
                    delivered_at=self.clock.now(),
                    extra={"attempts": attempt},
                )
            if 400 <= status < 500:
                return DispatchResult(
                    ok=False, channel=self._name,
                    error=f"status={status} client error: {text[:200]!r}",
                )
            last_err = f"status={status}, body={text[:200]!r}"
            await self._sleep(attempt)
        return DispatchResult(
            ok=False, channel=self._name, error=last_err or "unknown",
        )

    async def _sleep(self, attempt: int) -> None:
        if attempt >= self.max_retries:
            return
        await asyncio.sleep(self.backoff_s * (2 ** (attempt - 1)))


__all__ = ["WebhookChannel", "WebhookPoster"]
