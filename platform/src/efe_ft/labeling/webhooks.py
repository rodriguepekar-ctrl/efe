"""Outbound webhook dispatcher for labeling events.

Each tenant may configure a webhook endpoint. When a label is submitted,
a dual-label disagreement is escalated, or an annotator's κ drops, a
signed POST is fired. The dispatcher handles retries with exp backoff
and routes failures into a dead-letter queue.
"""

from __future__ import annotations

import asyncio
import hashlib
import hmac
import json
import logging
import time
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Any

_LOG = logging.getLogger(__name__)


@dataclass(frozen=True, slots=True)
class WebhookEvent:
    event: str
    tenant_id: str
    payload: dict[str, Any]
    event_id: str

    def canonical_body(self) -> bytes:
        return json.dumps({
            "event_id": self.event_id,
            "event": self.event,
            "tenant_id": self.tenant_id,
            "payload": self.payload,
            "ts": time.time(),
        }, sort_keys=True, separators=(",", ":")).encode("utf-8")


HttpPoster = Callable[[str, bytes, dict[str, str]], Awaitable[int]]


class LabelingWebhookDispatcher:
    """Sign + POST labeling events with retries."""

    def __init__(
        self,
        *,
        endpoint_for_tenant: Callable[[str], tuple[str, str] | None],
        poster: HttpPoster,
        max_retries: int = 3,
        initial_backoff_s: float = 1.0,
    ) -> None:
        self._endpoint = endpoint_for_tenant
        self._poster = poster
        self._max_retries = max_retries
        self._initial_backoff = initial_backoff_s
        self._dead_letter: list[tuple[WebhookEvent, str]] = []

    @property
    def dead_letter(self) -> list[tuple[WebhookEvent, str]]:
        return list(self._dead_letter)

    async def send(self, event: WebhookEvent) -> bool:
        config = self._endpoint(event.tenant_id)
        if config is None:
            return False
        url, secret = config
        body = event.canonical_body()
        signature = hmac.new(
            secret.encode("utf-8"), body, hashlib.sha256,
        ).hexdigest()
        headers = {
            "content-type": "application/json",
            "x-efe-ft-event": event.event,
            "x-efe-ft-event-id": event.event_id,
            "x-efe-ft-signature": f"sha256={signature}",
        }
        delay = self._initial_backoff
        last_err: str | None = None
        for attempt in range(self._max_retries + 1):
            try:
                status = await self._poster(url, body, headers)
            except Exception as exc:
                last_err = f"{type(exc).__name__}: {exc}"
                status = 0
            if 200 <= status < 300:
                return True
            _LOG.warning(
                "webhook failed attempt=%d status=%s", attempt + 1, status,
            )
            last_err = last_err or f"http {status}"
            if attempt < self._max_retries:
                await asyncio.sleep(delay)
                delay *= 2
        self._dead_letter.append((event, last_err or "unknown"))
        return False


__all__ = ["LabelingWebhookDispatcher", "WebhookEvent"]
