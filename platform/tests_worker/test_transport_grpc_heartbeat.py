"""Tests for :mod:`efe_worker.transport.grpc_client` + heartbeat loop."""

from __future__ import annotations

import asyncio

import pytest

from efe_worker.context import RunContext
from efe_worker.errors import OrchestratorUnreachable
from efe_worker.transport.grpc_client import (
    CheckpointRequest,
    ErrorRequest,
    FakeOrchestratorClient,
    HeartbeatRequest,
    ResultRequest,
)
from efe_worker.transport.heartbeat import heartbeat_loop


async def test_fake_heartbeat_records() -> None:
    c = FakeOrchestratorClient()
    ok = await c.heartbeat(HeartbeatRequest(
        run_id="r", worker_id="w", beat_seq=1,
    ))
    assert ok is True
    assert len(c.heartbeats) == 1


async def test_fake_checkpoint() -> None:
    c = FakeOrchestratorClient()
    await c.report_checkpoint(CheckpointRequest(
        run_id="r", worker_id="w", step=10,
        blob_uri="s3://x", metrics={"loss": 0.1}, is_best=True,
    ))
    assert c.checkpoints[0].step == 10
    assert c.checkpoints[0].is_best


async def test_fake_result() -> None:
    c = FakeOrchestratorClient()
    await c.report_result(ResultRequest(
        run_id="r", worker_id="w", artifact_url="s3://a",
        metrics={"f1": 0.9},
    ))
    assert c.results[0].artifact_url == "s3://a"


async def test_fake_error() -> None:
    c = FakeOrchestratorClient()
    await c.report_error(ErrorRequest(
        run_id="r", worker_id="w", error_code="cuda_oom",
        error_message="OOM",
    ))
    assert c.errors[0].error_code == "cuda_oom"


async def test_fake_close_sets_closed() -> None:
    c = FakeOrchestratorClient()
    await c.close()
    assert c.closed is True


async def test_fake_failure_propagates() -> None:
    c = FakeOrchestratorClient(fail_heartbeat=True)
    with pytest.raises(OrchestratorUnreachable):
        await c.heartbeat(HeartbeatRequest(
            run_id="r", worker_id="w", beat_seq=1,
        ))


# --- heartbeat loop integration -------------------------------------


async def test_heartbeat_loop_stops_when_event_set(
    run_context: RunContext, fake_transport: FakeOrchestratorClient,
) -> None:
    # Very short interval so the loop ticks immediately.
    task = asyncio.create_task(
        heartbeat_loop(fake_transport, run_context, interval_s=0.01),
    )
    await asyncio.sleep(0.05)
    run_context.request_stop()
    sent = await asyncio.wait_for(task, timeout=1.0)
    assert sent >= 1
    # Each heartbeat carries the run identity.
    assert fake_transport.heartbeats[0].run_id == run_context.run_id
    assert fake_transport.heartbeats[0].worker_id == run_context.worker_id


async def test_heartbeat_loop_continues_on_transport_failure(
    run_context: RunContext,
) -> None:
    client = FakeOrchestratorClient(fail_heartbeat=True)
    task = asyncio.create_task(
        heartbeat_loop(client, run_context, interval_s=0.01),
    )
    await asyncio.sleep(0.05)
    run_context.request_stop()
    sent = await asyncio.wait_for(task, timeout=1.0)
    # No successful sends due to fail_heartbeat=True, but loop survived.
    assert sent == 0


async def test_heartbeat_carries_live_metrics(
    run_context: RunContext, fake_transport: FakeOrchestratorClient,
) -> None:
    run_context.record(step=3, loss=0.25, lr=1e-5)
    run_context.set_phase("training")
    task = asyncio.create_task(
        heartbeat_loop(fake_transport, run_context, interval_s=0.01),
    )
    await asyncio.sleep(0.03)
    run_context.request_stop()
    await asyncio.wait_for(task, timeout=1.0)
    hb = fake_transport.heartbeats[0]
    assert hb.step == 3
    assert hb.state_hint == "training"
    assert hb.extra["loss"] == 0.25
    assert hb.extra["lr"] == 1e-5
