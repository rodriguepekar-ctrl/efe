"""Async heartbeat loop.

Runs as a background task for the entire lifetime of the run. Each
tick:

    1. Build a :class:`HeartbeatRequest` from the current
       :class:`RunContext` state.
    2. Call :meth:`OrchestratorClient.heartbeat` with a strict
       timeout.
    3. On success, bump ``beat_seq``. On transport failure, log and
       continue — never raise. The orchestrator's reaper will
       eventually declare the worker lost if enough beats are missed.

The loop exits cleanly when ``ctx.stop_event`` is set (SIGTERM,
driver done, fatal error) and returns the number of beats sent.
"""

from __future__ import annotations

import asyncio

from efe_worker.context import RunContext
from efe_worker.observability.logging import get_logger
from efe_worker.transport.grpc_client import (
    HeartbeatRequest,
    OrchestratorClient,
)

_log = get_logger(__name__)


async def heartbeat_loop(
    client: OrchestratorClient,
    ctx: RunContext,
    *,
    interval_s: float | None = None,
) -> int:
    """Emit heartbeats until ``ctx.stop_event`` is set.

    Returns the total number of successful sends.
    """
    interval = interval_s or ctx.settings.transport.heartbeat_interval_s
    seq = 0
    sent = 0
    while not ctx.should_stop():
        seq += 1
        req = HeartbeatRequest(
            run_id=ctx.run_id,
            worker_id=ctx.worker_id,
            beat_seq=seq,
            state_hint=ctx.phase,
            progress_ratio=ctx.progress(),
            step=ctx.step,
            extra=dict(ctx.live_metrics),
        )
        try:
            await client.heartbeat(req)
            sent += 1
        except Exception as exc:
            _log.warning(
                "heartbeat_failed",
                seq=seq,
                error=str(exc),
                type=type(exc).__name__,
            )
        # Sleep via wait(stop_event) so we wake up immediately on stop.
        try:
            await asyncio.wait_for(ctx.stop_event.wait(), timeout=interval)
        except asyncio.TimeoutError:
            pass  # normal path — tick expired
    return sent


__all__ = ["heartbeat_loop"]
