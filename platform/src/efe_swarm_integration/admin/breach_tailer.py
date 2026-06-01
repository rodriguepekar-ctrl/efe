"""BreachTailer — poll :class:`AdminClient.breach_history` and emit events."""

from __future__ import annotations

import asyncio
import logging
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any, Awaitable, Callable

from efe_swarm_integration.admin.client import AdminClient
from efe_swarm_integration.contract.adapter_result import parse_breach_report
from efe_swarm_integration.types import BreachReport

_LOG = logging.getLogger("efe_swarm_integration.admin.breach_tailer")

__all__ = ["BreachTailer"]


BreachHandler = Callable[[BreachReport], Awaitable[None]]


@dataclass(slots=True)
class BreachTailer:
    admin: AdminClient
    tenant_id: str
    handler: BreachHandler
    poll_interval_s: float = 5.0
    min_magnitude: float = 0.0
    _stopping: bool = False
    _cursor: datetime | None = None
    _task: asyncio.Task | None = None

    async def run_once(self) -> int:
        """Poll and dispatch. Returns the number of reports handled."""
        reports_raw = await self.admin.breach_history(
            tenant_id=self.tenant_id,
            min_magnitude=self.min_magnitude,
            limit=100,
        )
        handled = 0
        for raw in reports_raw:
            try:
                report: BreachReport = parse_breach_report(raw)
            except Exception as exc:
                _LOG.warning("breach_tailer.bad_report err=%s", exc)
                continue
            if self._cursor is not None and report.timestamp <= self._cursor:
                continue
            try:
                await self.handler(report)
            except Exception:
                _LOG.exception(
                    "breach_tailer.handler_failed cone_key=%s",
                    report.cone_key,
                )
                continue
            if self._cursor is None or report.timestamp > self._cursor:
                self._cursor = report.timestamp
            handled += 1
        return handled

    async def run_forever(self) -> None:
        while not self._stopping:
            try:
                await self.run_once()
            except Exception:  # pragma: no cover
                _LOG.exception("breach_tailer.poll_failed")
            await asyncio.sleep(self.poll_interval_s)

    def start(self) -> None:
        if self._task is not None:
            return
        self._task = asyncio.create_task(self.run_forever())

    async def stop(self) -> None:
        self._stopping = True
        if self._task is not None:
            self._task.cancel()
            try:
                await self._task
            except (asyncio.CancelledError, Exception):  # pragma: no cover
                pass
            self._task = None

    @property
    def cursor(self) -> datetime | None:
        return self._cursor


_ = timezone, Any
