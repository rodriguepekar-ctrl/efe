"""RetryInterceptor — wraps a unary call with the configured policy."""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from typing import Awaitable, Callable

from efe_swarm_integration.flow_control.retry import (
    RetryClassifier,
    backoff_delay_ms,
)

__all__ = ["RetryInterceptor"]


@dataclass(slots=True)
class RetryInterceptor:
    classifier: RetryClassifier
    idempotent: bool = False

    async def invoke(
        self,
        *,
        call: Callable[[], Awaitable[object]],
        idempotent: bool | None = None,
    ) -> object:
        is_idem = self.idempotent if idempotent is None else idempotent
        attempt = 1
        while True:
            try:
                return await call()
            except _RetryableStatus as exc:
                if not self.classifier.should_retry(
                    status=exc.status, attempt=attempt, idempotent=is_idem
                ):
                    raise
                delay_ms = backoff_delay_ms(
                    attempt, cfg=self.classifier._cfg  # noqa: SLF001
                )
                await asyncio.sleep(delay_ms / 1000.0)
                attempt += 1


class _RetryableStatus(Exception):
    """Internal marker raised by drivers on retryable statuses ; the
    caller converts to the public Swarm* exception after exhaustion."""

    def __init__(self, status: str, *args: object) -> None:
        super().__init__(*args)
        self.status = status
